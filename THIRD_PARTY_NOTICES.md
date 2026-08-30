# Third-Party Notices

Feilian Lite is derived from
[PinkD/corplink-rs](https://github.com/PinkD/corplink-rs), initially imported
from commit `1ef93ae99be88951d9c0e7cc7b7e33b679fd5ed7`. That project is distributed
under GPL-2.0-only. Feilian Lite retains the upstream license in `license.txt`
and is distributed under the same terms.

The `libwg/wireguard-go` submodule was initially pinned to commit
`8936d2de0f2587fabeff9be7053eaef719d506d7` from
[PinkD/wireguard-go](https://github.com/PinkD/wireguard-go). Its license and
notices remain in the submodule.

Windows system-VPN builds include the official signed Wintun 0.14.1 AMD64 DLL,
downloaded from `wintun.net` during packaging and verified against SHA-256
`07c256185d6ee3652e09fa55c0b673e2624b565e02c4b9091c79ca7d2f24ef51`.
The prebuilt binary license is included at `third_party/wintun/LICENSE.txt` and
is copied into Windows release artifacts.