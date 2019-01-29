# ALSA/NAD receiver volume sync

Synchronize volume of an ALSA mixer with a NAD receiver using TCP API. NAD D7050 and NAD C338 should work.

## Usage

```shell
cargo build --release
./target/release/nad-volume-sync <MIXER NAME> <RECEIVER IP ADDRESS>
```

The mixer device should be a null device. For example, to create a null mixer using PulseAudio:

```
pactl load-module module-null-sink sink_name=receiver
```

Assuming receiver is running IP address `10.0.1.2`. Then, start volume control

```
./target/release/nad-volume-sync receiver 10.0.1.2
```
