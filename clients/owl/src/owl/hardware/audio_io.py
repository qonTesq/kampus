"""USB microphone + system/Bluetooth output, with streaming resampling.

Echo strategy is deliberately half-duplex: the app sends silence while the owl
is replying. No claim of acoustic echo cancellation or voice barge-in.
"""
import queue
import threading
import time
import numpy as np

def device_value(value):
    return None if value in (-1, "", None) else value

class AudioIO:
    def __init__(self, cfg):
        import sounddevice as sd
        import soxr
        self.cfg = cfg
        self.input_queue = queue.Queue(maxsize=100)
        self.output_buffer = bytearray()
        self.lock = threading.Lock()
        self.audible_until = 0.0
        self.level = 0.0
        self.error = None
        self.input_drops = 0
        self.input_status_count = 0
        self.output_status_count = 0
        self.resampler = soxr.ResampleStream(24000, cfg["output_rate"], 1, dtype="int16", quality="HQ")
        self.input = sd.RawInputStream(
            samplerate=cfg["input_rate"], channels=cfg["input_channels"], dtype="int16",
            device=device_value(cfg["input_device"]),
            blocksize=cfg["input_rate"] * cfg["block_ms"] // 1000,
            callback=self._capture)
        self.output = sd.RawOutputStream(
            samplerate=cfg["output_rate"], channels=cfg["output_channels"], dtype="int16",
            device=device_value(cfg["output_device"]), blocksize=0,
            latency="high", callback=self._play)

    def _capture(self, data, frames, time_info, status):
        if status: self.input_status_count += 1
        samples = np.frombuffer(data, dtype="<i2").reshape(-1, self.cfg["input_channels"])
        mono = samples[:, self.cfg["mic_channel"]].copy()
        item = (time.monotonic(), mono)
        try: self.input_queue.put_nowait(item)
        except queue.Full:
            self.input_drops += 1
            try: self.input_queue.get_nowait()
            except queue.Empty: pass
            try: self.input_queue.put_nowait(item)
            except queue.Full: pass

    def _play(self, outdata, frames, time_info, status):
        if status: self.output_status_count += 1
        size = frames * self.cfg["output_channels"] * 2
        with self.lock:
            take = min(size, len(self.output_buffer))
            chunk = bytes(self.output_buffer[:take])
            del self.output_buffer[:take]
        outdata[:] = chunk + b"\0" * (size-take)
        if take:
            samples = np.frombuffer(chunk, dtype="<i2").astype(np.float32)
            self.level = min(1.0, float(np.sqrt(np.mean(samples*samples))) / 7000)
            # Account for PortAudio scheduling plus an adjustable Bluetooth tail.
            latency = max(0, time_info.outputBufferDacTime - time_info.currentTime)
            self.audible_until = time.monotonic() + latency + frames/self.cfg["output_rate"] + self.cfg["echo_guard_seconds"]
        else:
            self.level *= .7

    def start(self):
        self.output.start()
        self.input.start()

    def playing(self):
        with self.lock:
            buffered = bool(self.output_buffer)
        return buffered or time.monotonic() < self.audible_until

    def feed_pcm24(self, pcm):
        samples = np.frombuffer(pcm, dtype="<i2")
        converted = self.resampler.resample_chunk(samples)
        converted = np.clip(converted.astype(np.float32) * self.cfg["volume"], -32768, 32767).astype("<i2")
        if self.cfg["output_channels"] > 1:
            converted = np.repeat(converted[:, None], self.cfg["output_channels"], axis=1)
        data = converted.tobytes()
        with self.lock:
            max_bytes = 45 * self.cfg["output_rate"] * self.cfg["output_channels"] * 2
            if len(self.output_buffer) + len(data) > max_bytes:
                raise RuntimeError("Speaker playback fell over 45 seconds behind; check the selected output device")
            self.output_buffer.extend(data)

    def clear_output(self):
        import soxr
        with self.lock: self.output_buffer.clear()
        self.resampler = soxr.ResampleStream(24000, self.cfg["output_rate"], 1, dtype="int16", quality="HQ")

    def read(self, timeout=.25):
        try: return self.input_queue.get(timeout=timeout)
        except queue.Empty: return None

    def drain_input(self):
        while True:
            try: self.input_queue.get_nowait()
            except queue.Empty: return

    def close(self):
        for stream in (self.input, self.output):
            try: stream.abort()
            except Exception: pass
            stream.close()
