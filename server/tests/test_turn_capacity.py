import unittest
from unittest.mock import patch
import deploy_turn_capacity


class CapacityTest(unittest.TestCase):
    def test_legacy_entry_cannot_bypass_managed_egress_budget(self):
        with patch.object(deploy_turn_capacity, 'deploy_management') as managed:
            deploy_turn_capacity.deploy()
        managed.assert_called_once()
        self.assertEqual(deploy_turn_capacity.LIMITS['bps-capacity'], 0)


if __name__ == '__main__':
    unittest.main()
