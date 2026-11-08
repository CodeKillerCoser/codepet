#!/bin/sh
set -eu
install -o root -g coturn -m 640 /etc/letsencrypt/live/${PUBLIC_IP}/fullchain.pem /etc/coturn/codepet-fullchain.pem
install -o root -g coturn -m 640 /etc/letsencrypt/live/${PUBLIC_IP}/privkey.pem /etc/coturn/codepet-privkey.pem
systemctl reload nginx || true
systemctl reload coturn || true
