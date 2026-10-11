"""Local wake-phrase listening with the offline Vosk model (no cloud)."""

import json
import re
from pathlib import Path

MODEL_NAME = "vosk-model-small-en-us-0.15"


def phrase_matches(text, phrases):
    words = " " + " ".join(re.findall(r"[\w']+", text.casefold())) + " "
    return any(" " + " ".join(re.findall(r"[\w']+", p.casefold())) + " " in words for p in phrases)


def download_model(cfg):
    import tempfile
    import zipfile
    from urllib.request import urlretrieve

    destination = Path(cfg["wake"]["model_path"])
    expected = MODEL_NAME
    if destination.name != expected:
        raise ValueError(
            "This helper downloads only the default English model; see README for other languages"
        )
    if destination.exists():
        print("Wake model already exists:", destination)
        return
    destination.parent.mkdir(parents=True, exist_ok=True)
    print("Downloading the official Vosk small English model (about 40 MB)...")
    with tempfile.TemporaryDirectory() as temp:
        archive = Path(temp) / "model.zip"
        urlretrieve("https://alphacephei.com/vosk/models/vosk-model-small-en-us-0.15.zip", archive)
        with zipfile.ZipFile(archive) as package:
            root = destination.parent.resolve()
            for member in package.infolist():
                target = (root / member.filename).resolve()
                if not target.is_relative_to(root) or member.filename.split("/")[0] != expected:
                    raise ValueError("Unexpected model archive path")
                if ((member.external_attr >> 16) & 0o170000) == 0o120000:
                    raise ValueError("Model archive must not contain symbolic links")
            package.extractall(root)
    print("Installed wake model:", destination)


def heard_wake_phrase(model, samples, rate, phrases):
    """Run the offline Vosk model on one utterance and look for the wake phrase."""
    import soxr
    from vosk import KaldiRecognizer

    rec = KaldiRecognizer(model, 16000, json.dumps(phrases + ["[unk]"]))
    rec.AcceptWaveform(soxr.resample(samples, rate, 16000).astype("<i2").tobytes())
    return phrase_matches(json.loads(rec.FinalResult()).get("text", ""), phrases)
