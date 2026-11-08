limit_req_zone $binary_remote_addr zone=codepet_ip:10m rate=${NGINX_RATE}r/s;
server {
 listen ${HTTPS_PORT} ssl;
 server_name ${PUBLIC_IP};
 ssl_certificate /etc/letsencrypt/live/${PUBLIC_IP}/fullchain.pem;
 ssl_certificate_key /etc/letsencrypt/live/${PUBLIC_IP}/privkey.pem;
 ssl_protocols TLSv1.2 TLSv1.3;
 client_max_body_size 100k;
 client_body_timeout 10s;
 access_log off;
 # Optional per-release download routes are maintained on the server.
 include /etc/nginx/codepet-downloads/*.conf;
 location / {
  limit_req zone=codepet_ip burst=${NGINX_BURST} nodelay;
  proxy_pass http://127.0.0.1:8787;
  proxy_set_header X-Real-IP $remote_addr;
  proxy_set_header X-Forwarded-For "";
  proxy_set_header Forwarded "";
  limit_req_status 429;
  proxy_read_timeout 20s;
  proxy_set_header Host $host;
 }
}
