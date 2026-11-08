"""Compatibility entry point; keep enrollment and all managed settings in one bundle."""
import sys
from deploy_management import deploy


if __name__ == '__main__':
    deploy(sys.argv[1])
