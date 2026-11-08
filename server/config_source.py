"""Read versioned VPS settings/templates; never import or export server secrets."""
import copy
import ipaddress
import json
from pathlib import Path
import re

from policy import Limits

CONFIG_DIR = Path(__file__).resolve().parent / 'config'


def read_config(name):
    return (CONFIG_DIR / name).read_text()


def render(name, **values):
    def replace(match):
        key = match[1]
        if key not in values:
            raise ValueError('Missing configuration placeholder: ' + key)
        return str(values[key])
    # Only explicit uppercase ${...} placeholders belong to this renderer.
    # nginx $host/$remote_addr and shell variables must remain untouched.
    return re.sub(r'\$\{([A-Z][A-Z0-9_]*)\}', replace, read_config(name))


def load_deployment():
    value = json.loads(read_config('deployment.json'))
    if set(value) != {'publicIp', 'httpsPort', 'nginx', 'turn', 'egress'}:
        raise ValueError('Unknown deployment setting')
    ipaddress.ip_address(value['publicIp'])
    expected_turn = {'listening-port', 'tls-listening-port', 'min-port', 'max-port',
                     'user-quota', 'total-quota', 'max-bps', 'bps-capacity'}
    if (set(value['turn']) != expected_turn or set(value['nginx']) != {'requestsPerSecond', 'burst'}
            or set(value['egress']) != {'bytesPerSecond', 'burstBytes'}):
        raise ValueError('Missing or unknown service setting')
    numbers = [value['httpsPort'], *value['turn'].values(), *value['nginx'].values(), *value['egress'].values()]
    if any(type(n) is not int or n < 0 for n in numbers):
        raise ValueError('Service settings must be non-negative integers')
    if any(n == 0 for n in [value['httpsPort'], *value['nginx'].values(), *value['egress'].values(),
                            *(v for k, v in value['turn'].items() if k != 'bps-capacity')]):
        raise ValueError('Service settings must be positive except bandwidth reservation')
    ports = [value['httpsPort'], *(value['turn'][key] for key in
             ('listening-port', 'tls-listening-port', 'min-port', 'max-port'))]
    if any(p > 65535 for p in ports) or value['turn']['min-port'] > value['turn']['max-port']:
        raise ValueError('Invalid service port')
    return value


DEPLOYMENT = load_deployment()


def signaling_defaults():
    value = json.loads(read_config('signaling.json'))
    if set(value) != {'database', 'deviceRegistration', 'maxDevices', 'trustedProxies', 'limits'}:
        raise ValueError('Only non-secret signaling defaults belong in the repository')
    Limits.parse(value['limits'])
    if type(value['maxDevices']) is not int or value['maxDevices'] <= 0:
        raise ValueError('Invalid device registration capacity')
    if type(value['deviceRegistration']) is not bool:
        raise ValueError('Invalid registration switch')
    for network in value['trustedProxies']:
        ipaddress.ip_network(network)
    return value


def configure_signal(existing):
    updated = copy.deepcopy(existing)
    defaults = signaling_defaults()
    # Keep identities and the database location of an existing installation.
    defaults['database'] = existing.get('database', defaults['database'])
    updated.update(defaults)
    return updated


def template_values(public_ip=None, turn_secret=None):
    public_ip = public_ip or DEPLOYMENT['publicIp']
    ipaddress.ip_address(public_ip)
    values = {
        'PUBLIC_IP': public_ip, 'HTTPS_PORT': DEPLOYMENT['httpsPort'],
        'NGINX_RATE': DEPLOYMENT['nginx']['requestsPerSecond'],
        'NGINX_BURST': DEPLOYMENT['nginx']['burst'],
        'TURN_OPTIONS': '\n'.join(f'{k}={v}' for k, v in DEPLOYMENT['turn'].items()),
    }
    if turn_secret is not None:
        if not re.fullmatch(r'[A-Za-z0-9+/=_-]{32,256}', turn_secret):
            raise ValueError('Invalid TURN secret format')
        values['TURN_SECRET'] = turn_secret
    return values


def firewall_ports():
    turn = DEPLOYMENT['turn']
    return [f"{DEPLOYMENT['httpsPort']}/tcp", f"{turn['listening-port']}/udp",
            f"{turn['listening-port']}/tcp", f"{turn['tls-listening-port']}/tcp",
            f"{turn['min-port']}-{turn['max-port']}/udp"]
