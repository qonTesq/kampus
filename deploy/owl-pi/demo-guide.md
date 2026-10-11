===============================================================================
  KAMPUS OWL - DEMO GUIDE
  How to set up, start, run and recover the talking owl during a demo.
  Brain: Gemini 3.8 Live (hears + thinks)   Voice: ElevenLabs   Eyes: 2 screens
===============================================================================

-------------------------------------------------------------------------------
THE DAY BEFORE (10 min)
-------------------------------------------------------------------------------
  [ ] Charge: JBL Go 3 and the iPhone (the hotspot drains the phone fast).
  [ ] Do one full rehearsal with "START THE DEMO" below, in a room like the
      demo room.
  [ ] ElevenLabs credit: the free plan is about 20 minutes of owl speech per
      month. Check what's left at elevenlabs.io (bottom-left of the app).
      The owl prints "ElevenLabs characters used this run" after each reply.
  [ ] Bring: the Pi's official 27W USB-C power supply, a laptop, and a
      charging cable for the phone.

-------------------------------------------------------------------------------
SET UP AT THE VENUE (5 min)
-------------------------------------------------------------------------------
  1. iPhone: Settings -> Personal Hotspot -> "Allow Others to Join" ON,
     "Maximize Compatibility" ON. Keep that screen OPEN until step 4 works.
  2. Plug the power supply into the Pi. Wait 1 minute (it boots by itself).
  3. Turn on the JBL Go 3.
  4. Laptop: connect Wi-Fi to  kampus-hotspot  (NOT eduroam), open
     PowerShell and log in:

        ssh hienle@172.20.10.8

     (If that doesn't answer:  ssh hienle@hienle.local )

-------------------------------------------------------------------------------
START THE DEMO (1 command)
-------------------------------------------------------------------------------

        cd ~/kampus/owl-pi
        ./run_owl.sh

  The launcher checks everything first and prints [ok] for each item:
  API keys, internet, JBL (reconnects it and sets the volume), webcam mic,
  power and temperature. If a line says [FAIL], do what it says and run
  ./run_owl.sh again.

  Then:
    - It says "Calibrating background noise, stay quiet for 2 s" -> stay quiet.
    - "Sleeping. Say 'hey owl'..." -> the eyes are closed. Ready.

-------------------------------------------------------------------------------
HOW TO TALK TO THE OWL
-------------------------------------------------------------------------------
  1. Say clearly, facing the webcam:   "Hey Owl"   (or "Hello Owl")
     -> eyes open, it says "Hoo! I'm awake."
  2. Ask one question, then STOP talking.
     -> eyes look surprised while it hears you, then "thinking"
     -> it starts answering about 2-3 seconds after you stop
     -> eyes show an expression (happy, surprised, explaining...)
  3. WAIT until it finishes AND the eyes blink once. The blink means
     "your turn": the mic is back on. The owl can't hear you while it
     talks, so it can't be interrupted.
  4. Say "goodbye" to put it to sleep (or it sleeps after 5 min of silence).
     Say "Hey Owl" again to wake it. Each wake starts a fresh conversation.

  Good demo questions (short answers, reliable):
     "Who are you?"
     "Tell me a fun fact about owls."
     "Can you explain what a Raspberry Pi is, in one sentence?"
     "Give me a tip for studying for exams."
     "Tell me a joke."
  Keep questions short and one at a time. Long rambling questions or two
  people talking at once confuse it.

-------------------------------------------------------------------------------
BACKUP MODES (if something goes wrong on stage)
-------------------------------------------------------------------------------
  "Hey Owl" isn't recognized (loud room)  ->  Ctrl+C, then:
        ./run_owl.sh --no-wake          (starts awake, no wake phrase)

  The room is too loud for the mic at all  ->  Ctrl+C, then:
        ./run_owl.sh --text             (type questions on the laptop;
                                          the owl still speaks + shows eyes)

  Eyes only (no internet needed):
        .venv/bin/python main.py --demo

-------------------------------------------------------------------------------
TROUBLESHOOTING
-------------------------------------------------------------------------------
  Problem                               Fix
  ------------------------------------  --------------------------------------
  ssh doesn't connect                   Laptop on kampus-hotspot? Hotspot
                                        screen open on the iPhone? Pi on for
                                        1 minute?
  [FAIL] JBL not connected              Press the JBL power button, run
                                        ./run_owl.sh again. (The JBL turns
                                        itself off when idle.)
  [FAIL] no internet                    Hotspot off or phone locked: open the
                                        Personal Hotspot screen again.
  Owl answers but no sound              JBL volume: press its + button.
  Owl doesn't react when you talk       Speak louder/closer to the webcam;
                                        wait for the blink before talking.
  Owl answers its own voice / cuts in   Move the JBL farther from the webcam
                                        (at least 30 cm, not facing it).
  "Cloud request failed"                Usually the hotspot dropped. Check the
                                        phone, then just ask again.
  Owl says the ElevenLabs quota is out  Free credit used up for the month:
                                        use a second ElevenLabs account key,
                                        or upgrade.
  Eyes frozen / program crashed         Ctrl+C, then ./run_owl.sh again.
  Pi unplugged by accident              See RESTART_AFTER_POWER_LOSS.txt

-------------------------------------------------------------------------------
AFTER THE DEMO
-------------------------------------------------------------------------------
  Ctrl+C stops the owl. Always shut down before unplugging:

        sudo shutdown -h now

  Wait for the green LED to stop blinking, then unplug.

-------------------------------------------------------------------------------
REFERENCE
-------------------------------------------------------------------------------
  Keys (on the Pi only, never in chat or GitHub):  nano ~/kampus/owl-pi/.env
  Settings (voice, model, volume):                 ~/kampus/owl-pi/config.toml
  Brain options in config.toml [pipeline] brain =  "gemini-live" (demo)
                                                   "gemini" / "openai"
===============================================================================
