"""Compatibility entry point: TURN admission and actual egress must deploy together."""
from pathlib import Path
from deploy_management import TURN_LIMITS as LIMITS, configure_turn as configure, deploy as deploy_management


def deploy():
    deploy_management(Path(__file__).parent)


if __name__ == '__main__':
    deploy()
