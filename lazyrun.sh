#!/bin/bash

# ==========================================
# open.drop — 一键全自动部署脚本 (Lazy Edition)
# 包含：环境依赖、PM2 守护、CLI 工具、Nginx 反向代理（5GB 上传支持）、SSL、BBR
# 参考自 sync-station 的 lazyRun.sh
# ==========================================

set -e

if [ "$EUID" -ne 0 ]; then
  echo -e "\033[31m[错误] 请使用 root 权限执行此脚本 (sudo bash lazyrun.sh)\033[0m"
  exit 1
fi

APP_DIR="/opt/opendrop"
CLI_BIN="/usr/local/bin/opendrop"
REPO_URL="${OPENDROP_REPO:-https://github.com/MrTangLuyao/opendrop.git}"
APP_PORT="${OPENDROP_PORT:-3000}"

echo -e "\033[32m[1/6] 正在更新系统依赖...\033[0m"
apt-get update -y
apt-get install -y curl sudo build-essential git nginx certbot python3-certbot-nginx iptables-persistent

echo -e "\033[32m[2/6] 正在安装 Node.js 20.x (LTS)...\033[0m"
if ! command -v node >/dev/null 2>&1 || [ "$(node -p 'parseInt(process.versions.node,10)')" -lt 18 ]; then
  curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
  apt-get install -y nodejs
fi

echo -e "\033[32m[3/6] 正在安装进程守护工具 PM2...\033[0m"
npm install -g pm2

echo -e "\033[32m[4/6] 正在从 GitHub 拉取最新源码...\033[0m"
rm -rf "$APP_DIR"
git clone "$REPO_URL" "$APP_DIR"
cd "$APP_DIR"
# better-sqlite3 needs python+make at install time; build-essential covers it
npm install --omit=dev

mkdir -p "$APP_DIR/data/uploads"
chown -R root:root "$APP_DIR/data"

echo -e "\033[32m[5/6] 正在配置系统管理指令 (opendrop)...\033[0m"
cat << EOF > "$CLI_BIN"
#!/bin/bash
APP_DIR="$APP_DIR"
APP_PORT="$APP_PORT"
case "\$1" in
    start)
        PORT=\$APP_PORT pm2 start \$APP_DIR/server.js --name "opendrop" --max-memory-restart 800M --time
        pm2 save
        echo "服务已启动 (端口 \$APP_PORT)"
        ;;
    stop)
        pm2 stop opendrop
        echo "服务已停止"
        ;;
    restart)
        pm2 restart opendrop
        echo "服务已重启"
        ;;
    status)
        pm2 status opendrop
        ;;
    logs)
        pm2 logs opendrop
        ;;
    config)
        echo "当前限制 (环境变量, 修改后请 opendrop restart):"
        echo "  OPENDROP_MAX_UPLOAD_GB    = \${OPENDROP_MAX_UPLOAD_GB:-5}  (默认 5 GB / 包裹)"
        echo "  OPENDROP_MAX_STORAGE_GB   = \${OPENDROP_MAX_STORAGE_GB:-30} (默认 30 GB / 系统)"
        echo "  OPENDROP_MAX_EXPIRY_HOURS = \${OPENDROP_MAX_EXPIRY_HOURS:-168} (默认 168 小时 = 7 天)"
        echo "  PORT                      = \${PORT:-3000}"
        echo
        echo "示例：临时把系统容量改成 50 GB —"
        echo "  pm2 stop opendrop && OPENDROP_MAX_STORAGE_GB=50 pm2 start \$APP_DIR/server.js --name opendrop && pm2 save"
        ;;
    update)
        echo "正在拉取最新版本..."
        pm2 stop opendrop 2>/dev/null || true
        git -C \$APP_DIR pull
        npm install --omit=dev --prefix \$APP_DIR
        if pm2 describe opendrop > /dev/null 2>&1; then
            pm2 restart opendrop --update-env
        else
            PORT=\$APP_PORT pm2 start \$APP_DIR/server.js --name "opendrop" --max-memory-restart 800M --time
        fi
        pm2 save
        echo "更新完成，服务已重启。"
        ;;
    reset)
        read -p "确定要清空所有上传文件和账户吗？(y/n): " confirm
        if [[ "\$confirm" =~ ^[Yy]\$ ]]; then
            pm2 stop opendrop 2>/dev/null || true
            rm -rf \$APP_DIR/data/uploads/*
            rm -f \$APP_DIR/data/db.json \$APP_DIR/data/db.json.tmp
            pm2 restart opendrop || PORT=\$APP_PORT pm2 start \$APP_DIR/server.js --name "opendrop"
            echo "数据已清空。"
        fi
        ;;
    uninstall)
        read -p "确定要彻底卸载吗？(y/n): " confirm
        if [[ "\$confirm" =~ ^[Yy]\$ ]]; then
            pm2 delete opendrop 2>/dev/null || true
            pm2 save
            rm -rf \$APP_DIR
            rm -f /usr/local/bin/opendrop
            echo "卸载完成。"
        fi
        ;;
    *)
        echo "用法: opendrop {start|stop|restart|status|logs|config|update|reset|uninstall}"
        ;;
esac
EOF
chmod +x "$CLI_BIN"

echo -e "\033[32m[6/6] 启动守护进程并配置自启...\033[0m"
"$CLI_BIN" start
pm2 startup | grep "sudo env" | bash || true
pm2 save

echo -e "\n\033[33m-------------------------------------------------------\033[0m"
read -p "是否开启 BBR+FQ 网络加速？(y/n): " CONFIG_BBR
if [[ "$CONFIG_BBR" =~ ^[Yy]$ ]]; then
    echo "正在写入系统内核参数..."
    sed -i '/net.core.default_qdisc/d' /etc/sysctl.conf
    sed -i '/net.ipv4.tcp_congestion_control/d' /etc/sysctl.conf
    echo "net.core.default_qdisc=fq" >> /etc/sysctl.conf
    echo "net.ipv4.tcp_congestion_control=bbr" >> /etc/sysctl.conf
    sysctl -p
    echo "BBR 已开启。"
fi

echo -e "\n\033[33m-------------------------------------------------------\033[0m"
read -p "是否现在配置域名反向代理和 HTTPS? (y/n): " CONFIG_DOMAIN
if [[ "$CONFIG_DOMAIN" =~ ^[Yy]$ ]]; then
    read -p "请输入域名（例如 drop.example.com）: " DOMAIN
    read -p "请输入联系邮箱（用于申请 SSL 证书）: " EMAIL

    echo "正在配置 Nginx (client_max_body_size 5GB，关闭代理缓冲以避免大文件白屏)..."
    cat << EOF > /etc/nginx/sites-available/opendrop
server {
    listen 80;
    server_name $DOMAIN;

    # open.drop 单次上传上限是 5 GB，nginx 必须放行同样大小
    client_max_body_size      5G;
    client_body_buffer_size   1m;
    client_body_timeout       600s;
    proxy_read_timeout        600s;
    proxy_send_timeout        600s;
    send_timeout              600s;

    location / {
        proxy_pass http://127.0.0.1:$APP_PORT;
        proxy_http_version 1.1;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;

        # 关闭缓冲，避免上传/下载 GB 级文件时 Nginx 把整个 body 先缓存到磁盘
        proxy_buffering         off;
        proxy_request_buffering off;
        proxy_cache_bypass      \$http_upgrade;
    }
}
EOF

    ln -sf /etc/nginx/sites-available/opendrop /etc/nginx/sites-enabled/
    rm -f /etc/nginx/sites-enabled/default
    nginx -t && systemctl reload nginx

    echo "正在申请 SSL 证书 (Certbot)..."
    if certbot --nginx -d "$DOMAIN" --non-interactive --agree-tos -m "$EMAIL"; then
        echo -e "\n\033[32m[成功] HTTPS 已开启：https://$DOMAIN\033[0m"
    else
        echo -e "\n\033[31m[警告] SSL 证书申请失败！\033[0m"
        echo -e "\033[33m请检查：\033[0m"
        echo "  1. 云平台安全组放行 TCP 80 / 443"
        echo "  2. 防火墙放行端口："
        echo "       sudo iptables -I INPUT 1 -p tcp --dport 80 -j ACCEPT"
        echo "       sudo iptables -I INPUT 1 -p tcp --dport 443 -j ACCEPT"
        echo "       sudo netfilter-persistent save"
        echo "  3. 重新执行: sudo certbot --nginx -d $DOMAIN"
    fi
else
    echo -e "\n\033[33m跳过域名配置。请通过 http://<server-ip>:$APP_PORT 访问。\033[0m"
fi

echo -e "\n\033[32m部署完成。常用命令：\033[0m"
echo "  opendrop start | stop | restart | status | logs"
echo "  opendrop config            查看当前配置"
echo "  opendrop update            从 GitHub 拉取新版本并重启"
echo "  opendrop reset             清空所有上传文件和账户"
echo "  opendrop uninstall         彻底卸载"
