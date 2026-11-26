# Parts for one voice-controlled lamp

For the first build, use an ESP32-S3 with **16 MiB flash and 8 MiB PSRAM**,
one digital microphone, and one mono speaker. Both audio breakouts use I²S.
The amplifier converts digital samples and drives the speaker, so this set
needs neither an I²C control board nor a separate audio codec.

## Shopping list

- **1 controller.** Waveshare ESP32-S3-DEV-KIT-N16R8 with the PCB antenna
  and headers fitted. Check the selected variant for the exact N16R8 suffix.
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

## Wiring for the first bench test

With USB disconnected, wire by signal name. Choose exposed GPIOs using the
pinout for the exact board revision. On the Waveshare N16R8, GPIO35 through
GPIO37 are unavailable. [Board pinout][board]

- Connect module grounds to board GND. Power the INMP441 from 3V3 and tie
  L/R to ground for the left slot. The ESP supplies bit clock to SCK and word
  select to WS. Connect SD to the ESP's I²S receive data pin. The microphone
  sends signed 24-bit samples in 32-bit slots. [INMP441 datasheet][mic]
- The amp takes the same bit clock at BCLK and word select at LRC. Use a
  separate ESP transmit data pin for DIN. Power the breakout from the
  board's USB 5 V rail if available and begin at low volume. Wire the
  speaker across the amp's + and − outputs. Both are driven. [Amp pinout][pins]
- Connect a free GPIO through the 330 Ω resistor to the LED anode and its
  cathode to GND. Driving that pin high lights the lamp.

During bring-up, check the LED, listen to a generated tone, then inspect a
short mic capture. Use 16 kHz mono PCM for the speech interfaces. Capture the
left slot and scale to signed 16-bit samples. Listen and speak in alternating
turns to avoid transcribing the controller's own reply.

[board]: https://docs.waveshare.com/ESP32-S3-DEV-KIT-N8R8
[mic]: https://invensense.tdk.com/wp-content/uploads/2015/02/INMP441.pdf
[amp]: https://www.adafruit.com/product/3006
[pins]: https://learn.adafruit.com/adafruit-max98357-i2s-class-d-mono-amp/pinouts
[buy-board]: https://www.amazon.it/s?k=ESP32-S3-DEV-KIT-N16R8
[buy-mic]: https://www.amazon.it/s?k=INMP441+I2S
[buy-amp]: https://www.amazon.it/s?k=MAX98357A+I2S
[buy-speaker]: https://www.amazon.it/s?k=altoparlante+8+ohm+3W+fili
