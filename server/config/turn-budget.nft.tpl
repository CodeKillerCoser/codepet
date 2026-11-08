add table inet codepet_turn_budget
flush table inet codepet_turn_budget
add chain inet codepet_turn_budget output { type filter hook output priority -5; policy accept; }
add chain inet codepet_turn_budget turn_egress
add rule inet codepet_turn_budget output meta skuid "coturn" oifname != "lo" counter jump turn_egress
add rule inet codepet_turn_budget turn_egress limit rate over ${EGRESS_RATE} bytes/second burst ${EGRESS_BURST} bytes counter drop
