# Parts for one voice-controlled lamp

Use the **Espressif ESP32-S3-DevKitC-1-N32R16V** with **32 MiB flash and
16 MiB PSRAM**, one digital microphone and one mono speaker. Both audio
breakouts use I²S. The amplifier converts digital samples and drives the
speaker directly. See the [wiring diagram and pin map](wiring.md) for assembly.

## Shopping list

- **1 controller.** Order the DevKitC-1 development board with headers and
  the exact N32R16V suffix. Its module is marked ESP32-S3-WROOM-2-N32R16V.
  Flash stores firmware and models. PSRAM holds speech-engine working memory.
  [Board variants][board] · [Amazon.it board search][buy-board]
- **1 microphone.** An INMP441 I²S breakout with headers fitted and pins
  marked VDD, GND, SCK, WS, SD and L/R. A bare MEMS package is difficult
  to breadboard. [Microphone datasheet][mic] · [Amazon.it mic search][buy-mic]
- **1 amplifier.** A MAX98357A mono I²S breakout with headers and a fitted
  speaker terminal. Adafruit #3006 lists at US$5.95 before shipping and tax
  as a price reference. [Amplifier][amp] · [Amazon.it amp search][buy-amp]
- **1 speaker.** A passive 8 Ω, 3 W miniature speaker with two wire leads
  fits the amplifier's screw terminal. [Amazon.it speaker search][buy-speaker]
- **1 lamp.** One ordinary red LED and one 330 Ω resistor in series.
  Reuse parts from a starter kit.
- **Bench supplies.** One USB data cable matching the board connector, a
  solderless breadboard and jumper wires. Female-to-male jumpers also work
  if a wide development board covers both rows of a breadboard.

The shopping links search by part number. Check the variant, fitted headers
and pack quantity when comparing local prices.

## Peripheral support

The ESP32-S3 has two I²S controllers. One shared pair of clocks and two data
lines serve the microphone and amplifier. The pinned Rust HAL (`esp-hal`
1.2.2) provides I²S receive and transmit with DMA through its `unstable`
feature. The external LED uses a regular GPIO output. [I²S guide][i2s]

Both hardware I²C controllers are available for future sensors. The HAL
provides an I²C master driver, and the chip supports 100 and 400 kHz.
The [pin map](wiring.md#optional-i2c-sensor) leaves a pair of GPIOs free
for SDA and SCL. [I²C guide][i2c]

[board]: https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32s3/esp32-s3-devkitc-1/user_guide_v1.1.html
[mic]: https://invensense.tdk.com/wp-content/uploads/2015/02/INMP441.pdf
[amp]: https://www.adafruit.com/product/3006
[i2s]: https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/api-reference/peripherals/i2s.html
[i2c]: https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/api-reference/peripherals/i2c.html
[buy-board]: https://www.amazon.it/s?k=ESP32-S3-DevKitC-1-N32R16V
[buy-mic]: https://www.amazon.it/s?k=INMP441+I2S
[buy-amp]: https://www.amazon.it/s?k=MAX98357A+I2S
[buy-speaker]: https://www.amazon.it/s?k=altoparlante+8+ohm+3W+fili
