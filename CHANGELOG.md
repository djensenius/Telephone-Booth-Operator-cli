# Changelog

## [0.5.0](https://github.com/djensenius/Telephone-Booth-Operator-cli/compare/v0.4.1...v0.5.0) (2026-07-19)


### Features

* label moderation as advisory AI suggestion, human decides ([#92](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/92)) ([acc6ee5](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/acc6ee5ab9d3991b2100e2decb6e59ad2bd9187b))

## [0.4.1](https://github.com/djensenius/Telephone-Booth-Operator-cli/compare/v0.4.0...v0.4.1) (2026-07-18)


### Bug Fixes

* bump ratatui to 0.30 to drop unsound lru and unmaintained paste ([#90](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/90)) ([0607b33](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/0607b337733ae152061641825d9eafa259226294))
* uppercase screen nav keys and gate UI behind login ([#88](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/88)) ([c752af3](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/c752af31f75a3652987fe7d66553a875cc80bb58))

## [0.4.0](https://github.com/djensenius/Telephone-Booth-Operator-cli/compare/v0.3.0...v0.4.0) (2026-07-17)


### Features

* admin tier, data export/import, and advanced-metrics client ([#86](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/86)) ([f13c78c](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/f13c78c7123ffb9c9c968f63dd0f69c80b5c3ca3))
* publish Linux Homebrew tarballs and multi-platform formula ([#83](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/83)) ([1da6f90](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/1da6f90ee43ce944a4d0bc4c9ee6bd80dd5ff042))
* **tui:** gate API tokens and debug screens behind admin tier ([#87](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/87)) ([557c4cd](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/557c4cde14214307867d46a1892c01a6dc4157a0))

## [0.3.0](https://github.com/djensenius/Telephone-Booth-Operator-cli/compare/v0.2.3...v0.3.0) (2026-07-17)


### Features

* live booth status over the operator WebSocket ([#81](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/81)) ([6117518](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/6117518697c0b10487cd8a4d94a49605389a1120))

## [0.2.3](https://github.com/djensenius/Telephone-Booth-Operator-cli/compare/v0.2.2...v0.2.3) (2026-06-13)


### Bug Fixes

* improve operator settings and navigation ([a486b06](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/a486b06ec5cd16094c8fe1fdd389c78e7b4eca17))

## [0.2.2](https://github.com/djensenius/Telephone-Booth-Operator-cli/compare/v0.2.1...v0.2.2) (2026-06-12)


### Bug Fixes

* install ALSA for the x86_64 .deb cross-build ([#67](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/67)) ([0235eb5](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/0235eb5e240e3082c400307278343a1a3a90833e))

## [0.2.1](https://github.com/djensenius/Telephone-Booth-Operator-cli/compare/v0.2.0...v0.2.1) (2026-06-12)


### Bug Fixes

* add Authentik setup guide for the CLI ([#64](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/64)) ([e847791](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/e8477910d399c2ff76925215720c25f025e9ddab))

## [0.2.0](https://github.com/djensenius/Telephone-Booth-Operator-cli/compare/v0.1.0...v0.2.0) (2026-06-12)


### Features

* add Authentik device-code authentication flow ([#36](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/36)) ([29ea21a](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/29ea21a7788a4e4fc8c1ac5df395638f70bd7ff0))
* add booth Debug panel with REST-polled read views ([#56](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/56)) ([3624d86](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/3624d86261017ad52c92a42e6bdf9b31dc435d8e))
* add booth simulate controls to the Debug panel ([#58](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/58)) ([58a3cf2](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/58a3cf26170b612626aa4c0f7febd17e3e6bfb78))
* add btm-style System Health dashboard from booth /metrics ([#53](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/53)) ([49d3816](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/49d3816d343289aa348daced59e1ee79afc32696))
* add Events log read screen ([#46](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/46)) ([83f803b](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/83f803b479e688aae4f397a329dee1e87c36182f))
* add interactive device-code login screen ([#38](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/38)) ([ef27034](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/ef27034f840fa1cae20f9988474a160ba529d9fa)), closes [#11](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/11)
* add live Status screen backed by the operator API ([#40](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/40)) ([c0e5d47](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/c0e5d47c3985ef60482442203dc12595d8f6a915))
* add message moderation actions to the Messages screen ([#48](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/48)) ([6e318ba](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/6e318ba2671dc931467ab7cc7a011e65a9d15517))
* add Messages read screen with master-detail view ([#41](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/41)) ([ec85cab](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/ec85cab0fd95adc48b503269893cf06396ea6d2e)), closes [#14](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/14)
* add operator live system screen ([#45](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/45)) ([7af5416](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/7af5416e1cd96fd2e159f2e9ffd34b8bb80d7156)), closes [#19](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/19)
* add operator statistics dashboard screen ([#44](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/44)) ([15210d4](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/15210d47ce6b61556af55cc62938347ea3087988))
* add question management actions to the Questions screen ([#49](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/49)) ([b6120be](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/b6120beaae3bd20b4d55b5c1f6f1d936d5ad6af1))
* add Questions read screen with master-detail view ([#42](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/42)) ([77b971b](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/77b971b23ad59b89b2674fa35820d5e83572bde7)), closes [#15](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/15)
* add ratatui app shell with screen router and navigation ([#35](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/35)) ([244474e](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/244474e945a0988f0f5268e95c3f4b39e4db790f))
* add Sessions read screen with timeline detail ([#43](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/43)) ([f5104cb](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/f5104cbfd7b7af8f79466ce46dbc9d82054410d0)), closes [#17](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/17)
* add tbo-audio in-terminal FLAC playback engine ([#59](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/59)) ([bdfe7c6](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/bdfe7c624f0cf612989406315442598385901464))
* add the API Tokens screen with create, revoke, and usage ([#50](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/50)) ([ed7d028](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/ed7d02817e1578d3169cae11ff6093763ff52984))
* add theme switching, richer Settings, and an About screen ([#61](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/61)) ([e0745b9](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/e0745b91c623d3a9fa81cb016ffa65dc783e6a57)), closes [#28](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/28)
* add typed operator REST client ([#39](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/39)) ([e4c1a42](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/e4c1a42c76df590189bd9336c86821df50e1f69f))
* implement booth debug-server REST client ([#52](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/52)) ([5e121be](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/5e121be73e839442bf776a3fd5442564df5eee08))
* implement tbo-metrics Prometheus parser and time-series buffers ([#51](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/51)) ([dd428bf](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/dd428bf44f4a2124b53924e93dcf1f9e0a58b66d))
* live-tail the Events screen over SSE ([#47](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/47)) ([bb8c433](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/bb8c4331f56ece87bb17e28f959da159502ffd03))
* persist the auth session and refresh it proactively ([#37](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/37)) ([670d9cf](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/670d9cfdd1e0efca033b56aa0ca6885eeec20179))
* pin booth LAN TLS certificates by SHA-256 fingerprint ([#54](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/54)) ([ef0f53a](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/ef0f53a79e8e199f49770fa8d5a3e054a6ab4ded))
* play message and question audio in-terminal ([#60](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/60)) ([7c829fb](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/7c829fba785af8f615e74e21add7eae5872cbc78)), closes [#27](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/27)
* port operator API wire types, config, and errors to tbo-core ([#34](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/34)) ([26dc2ce](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/26dc2cebbe12aea167cd03903d392a8ab205f50a))
* stream booth telemetry over the debug-server WebSocket ([#55](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/55)) ([eec4816](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/eec481699f13c2d8441a845d3214fa1fb9e54ef3))
* stream live booth telemetry into the Debug panel ([#57](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/57)) ([11620ac](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/11620ac412c6afe4aed7f60441813258719b012e))


### Bug Fixes

* add operator HTTP integration tests, man page/completions, and docs ([#62](https://github.com/djensenius/Telephone-Booth-Operator-cli/issues/62)) ([e079df2](https://github.com/djensenius/Telephone-Booth-Operator-cli/commit/e079df2801d4252b261fa4defda1c47d532a7a6e))
