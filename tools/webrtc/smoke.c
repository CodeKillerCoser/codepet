#include <rtc/rtc.h>
#include <stdio.h>

int main(void) {
    rtcConfiguration configuration = {0};
    int peer = rtcCreatePeerConnection(&configuration);
    if (peer < 0) return 1;
    if (rtcDeletePeerConnection(peer) < 0) return 2;
    rtcCleanup();
    puts("Native WebRTC C ABI load/create/cleanup passed");
    return 0;
}
