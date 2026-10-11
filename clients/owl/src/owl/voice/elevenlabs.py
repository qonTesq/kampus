"""ElevenLabs text-to-speech, streamed straight into the speaker."""
import requests

ELEVENLABS = "https://api.elevenlabs.io/v1"


class Voice:
    """ElevenLabs text-to-speech, streamed straight into the speaker."""

    def __init__(self, cfg, key, voice_id):
        self.cfg = cfg
        self.voice_id = voice_id
        self.http = requests.Session()
        self.http.headers["xi-api-key"] = key
        self.chars_used = 0

    def speak(self, text, audio, on_first_audio=None):
        # pcm_24000 = raw 16-bit mono at 24 kHz, exactly what AudioIO.feed_pcm24 plays.
        params = {"output_format": "pcm_24000"}
        if self.cfg["pipeline"].get("tts_latency"):
            params["optimize_streaming_latency"] = self.cfg["pipeline"]["tts_latency"]
        for attempt in (1, 2):               # one quick retry if the hotspot hiccups
            try:
                r = self.http.post(f"{ELEVENLABS}/text-to-speech/{self.voice_id}/stream",
                                   params=params, stream=True, timeout=(5, 20),
                                   json={"text": text, "model_id": self.cfg["pipeline"]["tts_model"]})
                break
            except requests.ConnectionError:
                if attempt == 2:
                    raise
        r.raise_for_status()
        leftover = b""
        for chunk in r.iter_content(4800):
            chunk = leftover + chunk
            cut = len(chunk) - len(chunk) % 2
            leftover = chunk[cut:]
            if cut:
                audio.feed_pcm24(chunk[:cut])
                if on_first_audio:
                    on_first_audio()
                    on_first_audio = None
        self.chars_used += len(text)
