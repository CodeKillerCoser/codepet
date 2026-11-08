import unittest
from deploy_management import configure_turn, configure_nginx, budget_rules, TURN_LIMITS


class ManagementTest(unittest.TestCase):
    def test_configuration_preserves_auth_tls_and_private_network_restrictions(self):
        preserved = 'static-auth-secret=private\ncert=/etc/cert.pem\ndenied-peer-ip=10.0.0.0-10.255.255.255\n'
        old = preserved + '\n'.join(f'{key}=1' for key in TURN_LIMITS) + '\n'
        result = configure_turn(old)
        self.assertTrue(result.startswith(preserved))
        self.assertEqual(configure_turn(result), result)
        self.assertIn('bps-capacity=0\n', result)
        self.assertGreater(TURN_LIMITS['max-port'] - TURN_LIMITS['min-port'] + 1, TURN_LIMITS['total-quota'])
        with self.assertRaises(ValueError):
            configure_turn(old + 'user-quota=1\n')

    def test_proxy_source_is_overwritten_and_download_location_preserved(self):
        downloads = 'location /downloads/ { alias /srv/downloads/; }'
        old = '''limit_req_zone $binary_remote_addr zone=codepet_ip:10m rate=10r/s;
server {
location / { limit_req zone=codepet_ip burst=30 nodelay;
proxy_pass http://127.0.0.1:8787;
}
''' + downloads + '\n}'
        new = configure_nginx(old)
        self.assertIn(downloads, new)
        self.assertIn('proxy_set_header X-Real-IP $remote_addr;', new)
        self.assertEqual(configure_nginx(new), new)
        with self.assertRaises(ValueError):
            configure_nginx(new.replace('X-Real-IP $remote_addr', 'X-Real-IP $http_x_real_ip'))

    def test_network_budget_is_scoped_to_coturn_without_flushing_firewall(self):
        rules = budget_rules()
        self.assertNotIn('flush ruleset', rules)
        self.assertNotIn('firewalld', rules)
        self.assertIn('meta skuid "coturn"', rules)
        self.assertIn('oifname != "lo"', rules)
        self.assertIn('10000000 bytes/second', rules)
        with self.assertRaises(ValueError):
            budget_rules(0)
