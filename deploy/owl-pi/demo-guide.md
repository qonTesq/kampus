# Owl demo guide

How to set up, start, run and recover the talking owl during a demo. Brain: Gemini Live (hears and thinks). Voice: Gemini Live's own voice, or ElevenLabs if `[pipeline] voice = "elevenlabs"`. Eyes: two round screens.

## The day before (10 min)

- [ ] Charge the Bluetooth speaker (JBL Go 3) and the phone (the hotspot drains it fast).
- [ ] Do one full rehearsal with [Start the demo](#start-the-demo), in a room like the demo room.
- [ ] If the owl speaks with ElevenLabs: the free plan is about 20 minutes of owl speech a month. Check what's left at elevenlabs.io. The owl prints "ElevenLabs characters used this run" after each reply.
- [ ] Bring the Pi's official 27 W USB-C power supply, a laptop, and a charging cable for the phone.

## Set up at the venue (5 min)

1. Phone: Settings → Personal Hotspot → "Allow Others to Join" on, "Maximize Compatibility" on. Keep that screen **open** until step 4 works (iPhones hide the hotspot when the screen locks).
2. Plug the power supply into the Pi and wait a minute; it boots by itself.
3. Turn on the speaker.
4. Connect the laptop to the hotspot (not eduroam: campus Wi-Fi blocks laptop-to-Pi connections) and log in: `ssh <user>@<pi-address>`. On an iPhone hotspot the Pi is usually at `172.20.10.x`; `<pi-hostname>.local` also works.

## Start the demo

```bash
cd ~/kampus/clients/owl
scripts/run_owl.sh
```

The launcher prints `[ok]` for each check: API keys, internet, speaker (it reconnects it and sets the volume), webcam mic, power and temperature. If a line says `[FAIL]`, do what it says and run it again.

Then:

- "Calibrating background noise, stay quiet for 2 s": stay quiet.
- "Sleeping. Say 'hey owl'…": the eyes are closed. Ready.

## How to talk to the owl

1. Say clearly, facing the webcam: **"Hey Owl"** (or "Hello Owl"). The eyes open and it says "Hoo! I'm awake."
2. Ask one question, then stop talking. The eyes look surprised while it hears you, then thoughtful; it starts answering 2–3 seconds after you stop, with an expression to match.
3. Wait until it finishes **and the eyes blink once**. The blink means "your turn": the mic is back on. The owl can't hear you while it talks, so it can't be interrupted.
4. Say "goodbye" to put it to sleep (or it sleeps after 5 minutes of silence). "Hey Owl" wakes it again with a fresh conversation.

Good demo questions (short, reliable answers):

- "Who are you?"
- "Tell me a fun fact about owls."
- "Can you explain what a Raspberry Pi is, in one sentence?"
- "Give me a tip for studying for exams."
- "Tell me a joke."

Keep questions short and one at a time. Long rambling questions, or two people talking at once, confuse it.

## Backup modes

| Problem on stage | Do this (Ctrl+C first) |
|---|---|
| "Hey Owl" isn't recognized (loud room) | `scripts/run_owl.sh --no-wake` starts awake |
| The room is too loud for the mic at all | `scripts/run_owl.sh --text`: type questions on the laptop; the owl still speaks and shows its eyes |
| No internet at all | `uv run owl-realtime --demo`: eyes only |

## Troubleshooting

| Problem | Fix |
|---|---|
| ssh doesn't connect | Laptop on the hotspot? Hotspot screen open on the phone? Pi on for a minute? |
| `[FAIL] JBL not connected` | Press the speaker's power button and run the launcher again (it turns itself off when idle). |
| `[FAIL] no internet` | Hotspot off or phone locked: open the Personal Hotspot screen again. |
| Owl answers but no sound | Turn the speaker's volume up. |
| Owl doesn't react when you talk | Speak louder or closer to the webcam; wait for the blink before talking. |
| Owl answers its own voice or cuts in | Move the speaker at least 30 cm from the webcam, not facing it. |
| "Cloud request failed" | Usually the hotspot dropped. Check the phone, then ask again. |
| Owl says the ElevenLabs quota is out | The month's free credit is used up: use a second ElevenLabs account key, or upgrade. |
| Eyes frozen or the program crashed | Ctrl+C, then `scripts/run_owl.sh` again. |
| Pi unplugged by accident | [restart-after-power-loss.md](restart-after-power-loss.md) |

## After the demo

Ctrl+C stops the owl. Always shut down before unplugging, then wait for the green LED to stop blinking:

```bash
sudo shutdown -h now
```

## Reference

| What | Where |
|---|---|
| Keys (on the Pi only, never in chat or GitHub) | `~/kampus/clients/owl/.env` |
| Settings (voice, model, volume) | `~/kampus/clients/owl/config/config.toml` |
| Brain options, `[pipeline] brain =` | `"gemini-live"` (demo), `"gemini"`, `"openai"` |
