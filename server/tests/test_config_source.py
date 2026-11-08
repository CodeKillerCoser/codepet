import json
import unittest
from unittest.mock import patch

import config_source as config
from deploy_management import budget_rules, configure_nginx, configure_turn


class ConfigSourceTest(unittest.TestCase):
    def test_rendering_preserves_nginx_variables_and_requires_secret_injection(self):
        values = config.template_values('203.0.113.10')
        nginx = config.render('nginx.conf.tpl', **values)
        self.assertIn('server_name 203.0.113.10;', nginx)
        self.assertIn('Host $host;', nginx)
        self.assertIn('X-Real-IP $remote_addr;', nginx)
        self.assertNotIn('${', nginx)
        with self.assertRaises(ValueError):
            config.render('turnserver.conf.tpl', **values)
        turn = config.render('turnserver.conf.tpl', **config.template_values('203.0.113.10', 'x' * 48))
        self.assertIn('static-auth-secret=' + 'x' * 48, turn)
        self.assertNotIn('${', turn)
        with self.assertRaises(ValueError):
            config.template_values('203.0.113.10', 'x' * 48 + '\nno-auth')

    def test_settings_apply_without_replacing_server_identity_or_database(self):
        old = {'publicIp': '198.51.100.9', 'turnSecret': 'private-test-value',
               'database': '/custom/state.db', 'maxDevices': 1,
               'limits': {'pending_total': 1}, 'extension': 'keep'}
        before = json.loads(json.dumps(old))
        updated = config.configure_signal(old)
        self.assertEqual(updated['limits'], config.signaling_defaults()['limits'])
        self.assertEqual(updated['maxDevices'], config.signaling_defaults()['maxDevices'])
        for key in ('publicIp', 'turnSecret', 'database', 'extension'):
            self.assertEqual(updated[key], old[key])
        self.assertEqual(old, before)

    def test_initial_and_incremental_configs_use_same_policy(self):
        values = config.template_values('203.0.113.10', 'x' * 48)
        nginx = config.render('nginx.conf.tpl', **values)
        self.assertEqual(configure_nginx(nginx), nginx)
        turn = config.render('turnserver.conf.tpl', **values)
        self.assertEqual(configure_turn(turn), turn)
        ports = config.firewall_ports()
        self.assertIn(f"{config.DEPLOYMENT['httpsPort']}/tcp", ports)
        self.assertIn(f"{config.DEPLOYMENT['turn']['min-port']}-{config.DEPLOYMENT['turn']['max-port']}/udp", ports)
        self.assertIn(f"{config.DEPLOYMENT['egress']['bytesPerSecond']} bytes/second", budget_rules())

    def test_repository_signaling_config_cannot_contain_credentials(self):
        value = config.signaling_defaults()
        value['turnSecret'] = 'must-not-be-versioned'
        with patch.object(config, 'read_config', return_value=json.dumps(value)):
            with self.assertRaises(ValueError):
                config.signaling_defaults()

    def test_invalid_port_and_negative_limits_are_rejected_before_deploy(self):
        for change in ('port', 'rate'):
            value = json.loads(json.dumps(config.DEPLOYMENT))
            if change == 'port':
                value['turn']['max-port'] = 65536
            else:
                value['egress']['bytesPerSecond'] = -1
            with patch.object(config, 'read_config', return_value=json.dumps(value)):
                with self.assertRaises(ValueError):
                    config.load_deployment()
