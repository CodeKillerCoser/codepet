import unittest
from deploy_turn_capacity import configure, LIMITS

class CapacityTest(unittest.TestCase):
    def test_limits_are_consistent_and_other_settings_survive(self):
        old = 'user-quota=8\ntotal-quota=64\nmax-bps=2000000\nbps-capacity=10000000\nstatic-auth-secret=private\nlistening-port=3478\n'
        new = configure(old)
        self.assertIn('static-auth-secret=private\nlistening-port=3478\n', new)
        self.assertEqual(configure(new), new)
        self.assertGreaterEqual(LIMITS['total-quota'], 2*LIMITS['user-quota'])
        self.assertEqual(LIMITS['total-quota'] * LIMITS['max-bps'], LIMITS['bps-capacity'])
    def test_missing_or_duplicate_limit_refuses_deployment(self):
        with self.assertRaises(ValueError): configure('user-quota=8\nuser-quota=8\n')

if __name__ == '__main__': unittest.main()
