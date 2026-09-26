"""Record public fixtures. Python standard library only; cookies stay in memory."""
import argparse
import csv
import hashlib
import http.cookiejar
import io
import json
import re
from pathlib import Path
import tempfile
import time
from datetime import datetime, timezone
import urllib.error
import urllib.parse
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parent
PROVENANCE = "Public vendor response recorded for offline interoperability tests. Upstream retains rights; no additional license is asserted."


def digest(body):
    return hashlib.sha256(body).hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


class Recorder:
    def __init__(self):
        self.stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")

    def capture(self, vendor, label, url, opener=None, form=None):
        folder = ROOT / vendor / (self.stamp + "-" + label)
        folder.mkdir(parents=True, exist_ok=False)
        data = urllib.parse.urlencode(form).encode() if form is not None else None
        request = urllib.request.Request(url, data=data, headers={"User-Agent": "kampus-public-fixture-capture/1", "Accept-Encoding": "identity"})
        meta = {"format_version": 1, "kind": "recorded", "captured_at": datetime.now(timezone.utc).isoformat(),
                "source_url": url, "request": {"method": request.get_method(), "form": form, "headers": {"Accept-Encoding": "identity"}, "cookies": "omitted"},
                "license_provenance": PROVENANCE}
        body = None
        try:
            try:
                response = (opener or urllib.request.build_opener()).open(request, timeout=30)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                body = response.read()
                if vendor == "hours":
                    original_hash = digest(body)
                    body, count = re.subn(rb'(<meta name="csrf-token" content=")[^"]*("\s*/?>)', rb'\1[REDACTED]\2', body)
                    if count:
                        meta.update(redactions=["HTML csrf-token content"], original_sha256=original_hash)
                meta.update(http_status=response.status, content_type=response.headers.get("Content-Type", ""),
                            body_file="body.bin", sha256=digest(body), byte_length=len(body))
            (folder / "body.bin").write_bytes(body)
        except (OSError, urllib.error.URLError) as error:
            meta.update(http_status=None, content_type=None, transport_error=type(error).__name__)
        write_json(folder / "manifest.json", meta)
        print(f"{vendor}/{folder.name}: HTTP {meta['http_status']}, {len(body) if body is not None else 0} bytes", flush=True)
        time.sleep(1)
        return body, folder


def public(rec, cfg, date):
    base = cfg["temple_libcal"] + "/api/1.1/space/"
    rec.capture("libcal", "temple-locations", base + "locations")
    rec.capture("libcal", "temple-items", base + "items/" + cfg["location_id"])
    rec.capture("libcal", "temple-availability", base + "items/" + cfg["location_id"] + "?" + urllib.parse.urlencode({"availability": date}))
    rec.capture("libcal", "sju-locations", cfg["sju_libcal"] + "/api/1.1/space/locations")
    search = cfg["banner"] + "/searchResults/searchResults?" + urllib.parse.urlencode({"txt_term": cfg["term"], "txt_subject": cfg["subject"], "pageOffset": 0, "pageMaxSize": 10, "sortColumn": "subjectDescription", "sortDirection": "asc"})
    rec.capture("banner", "no-session", search)
    jar = http.cookiejar.CookieJar()
    session = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
    # Anonymous public search session only. Never serialize cookies or headers.
    rec.capture("banner", "term-session", cfg["banner"] + "/term/search?mode=search", session, {"term": cfg["term"]})
    rec.capture("banner", "search-with-session", search, session)
    jar.clear()
    for label, station in [("temple-arrivals", cfg["rail_station"]), ("subway-error", cfg["invalid_rail_station"])]:
        rec.capture("septa", label, cfg["rail"] + "?" + urllib.parse.urlencode({"station": station}))
    rec.capture("hours", "temple-hours", cfg["hours"])


def gtfs(rec, cfg):
    body, release_folder = rec.capture("septa", "gtfs-release", cfg["gtfs_release_api"])
    if body is None:
        raise ValueError("GTFS release lookup failed")
    release = json.loads(body)
    asset = next(a for a in release["assets"] if a["name"] in ("gtfs_public.zip", "google_bus.zip"))
    folder = ROOT / "septa" / (rec.stamp + "-gtfs-subset")
    folder.mkdir()
    meta = {"format_version": 1, "kind": "derived_gtfs_subset", "captured_at": datetime.now(timezone.utc).isoformat(),
            "source_url": asset["browser_download_url"], "source_release": release["tag_name"],
            "release_manifest": str(release_folder.relative_to(ROOT)).replace("\\", "/") + "/manifest.json",
            "stop_ids": cfg["gtfs_stop_ids"], "license_provenance": PROVENANCE}
    # Full archives and the 105 MB stop_times remain in a temporary directory.
    with tempfile.TemporaryDirectory(prefix="kampus-gtfs-") as temp:
        archive = Path(temp) / "feed.zip"
        request = urllib.request.Request(asset["browser_download_url"], headers={"User-Agent": "kampus-public-fixture-capture/1"})
        with urllib.request.urlopen(request, timeout=60) as response, archive.open("wb") as target:
            meta.update(http_status=response.status, content_type=response.headers.get("Content-Type", ""))
            while chunk := response.read(1024 * 1024):
                target.write(chunk)
        with archive.open("rb") as source:
            meta["source_sha256"] = hashlib.file_digest(source, "sha256").hexdigest()
        with zipfile.ZipFile(archive) as outer:
            nested = next((n for n in outer.namelist() if n.endswith("google_bus.zip")), None)
            if nested:
                bus = Path(temp) / "bus.zip"
                with outer.open(nested) as source, bus.open("wb") as target:
                    while chunk := source.read(1024 * 1024):
                        target.write(chunk)
                feed_path = bus
            else:
                feed_path = archive
        with zipfile.ZipFile(feed_path) as feed:
            meta["source_member_sha256"] = {n: hashlib.sha256(feed.read(n)).hexdigest() for n in feed.namelist() if n.endswith(".txt") and not n.endswith("stop_times.txt")}
            # Hash the large member by streaming, too.
            with feed.open("stop_times.txt") as stream:
                meta["source_member_sha256"]["stop_times.txt"] = hashlib.file_digest(stream, "sha256").hexdigest()

            def rows(name):
                with feed.open(name) as stream:
                    yield from csv.DictReader(io.TextIOWrapper(stream, encoding="utf-8-sig", newline=""))

            stops = [r for r in rows("stops.txt") if r["stop_id"] in cfg["gtfs_stop_ids"]]
            if {r["stop_id"] for r in stops} != set(cfg["gtfs_stop_ids"]):
                raise ValueError("Configured GTFS stop missing")
            parents = {r.get("parent_station") for r in stops} - {"", None}
            stops += [r for r in rows("stops.txt") if r["stop_id"] in parents and r["stop_id"] not in cfg["gtfs_stop_ids"]]
            times = [r for r in rows("stop_times.txt") if r["stop_id"] in cfg["gtfs_stop_ids"]]
            trip_ids = {r["trip_id"] for r in times}
            trips = [r for r in rows("trips.txt") if r["trip_id"] in trip_ids]
            services = {r["service_id"] for r in trips}
            routes = {r["route_id"] for r in trips}
            tables = {"stops.txt": stops, "stop_times.txt": times, "trips.txt": trips,
                      "routes.txt": [r for r in rows("routes.txt") if r["route_id"] in routes],
                      "agency.txt": list(rows("agency.txt"))}
            for name in ("calendar.txt", "calendar_dates.txt"):
                if name in feed.namelist():
                    tables[name] = [r for r in rows(name) if r["service_id"] in services]
            meta["files"] = {}
            for name, records in tables.items():
                # Preserve column names even when a calendar has no matching rows.
                with feed.open(name) as stream:
                    fields = next(csv.reader(io.TextIOWrapper(stream, encoding="utf-8-sig", newline="")))
                with (folder / name).open("w", encoding="utf-8", newline="") as target:
                    writer = csv.DictWriter(target, fieldnames=fields)
                    writer.writeheader()
                    writer.writerows(records)
                meta["files"][name] = {"rows": len(records), "sha256": digest((folder / name).read_bytes())}
            calendars = tables.get("calendar.txt", [])
            dates = [r["date"] for r in tables.get("calendar_dates.txt", []) if r["exception_type"] == "1"]
            starts = [r["start_date"] for r in calendars] + dates
            ends = [r["end_date"] for r in calendars] + dates
            meta["validity"] = {"start": min(starts), "end": max(ends)}
            meta["agency_timezones"] = sorted({r["agency_timezone"] for r in tables["agency.txt"]})
    write_json(folder / "manifest.json", meta)
    print(f"{folder.relative_to(ROOT)}: {len(times)} stop times, validity {meta['validity']}", flush=True)


def verify():
    manifests = list(ROOT.glob("*/*/manifest.json"))
    for path in manifests:
        meta = json.loads(path.read_text(encoding="utf-8"))
        for key in ("format_version", "kind", "captured_at", "source_url", "license_provenance"):
            assert key in meta, (path, key)
        datetime.fromisoformat(meta["captured_at"])
        assert meta["source_url"].startswith("https://"), path
        if meta["kind"] == "recorded":
            assert "request" in meta and "http_status" in meta and "content_type" in meta, path
            if meta["http_status"] is not None:
                body = (path.parent / meta["body_file"]).read_bytes()
                assert digest(body) == meta["sha256"] and len(body) == meta["byte_length"], path
            else:
                assert "transport_error" in meta, path
        else:
            for name, info in meta["files"].items():
                assert digest((path.parent / name).read_bytes()) == info["sha256"], path
    assert manifests, "No manifests"
    print(f"Verified {len(manifests)} manifests and body/subset checksums")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["public", "gtfs", "flight", "verify"])
    parser.add_argument("--config", type=Path, default=ROOT / "sources.json")
    parser.add_argument("--date", default=datetime.now().date().isoformat())
    parser.add_argument("--gtfs-release", help="Release tag instead of latest, e.g. v202609061")
    args = parser.parse_args()
    cfg = json.loads(args.config.read_text(encoding="utf-8"))
    if args.gtfs_release:
        cfg["gtfs_release_api"] = cfg["gtfs_release_api"].rsplit("/", 1)[0] + "/tags/" + urllib.parse.quote(args.gtfs_release, safe="")
    recorder = Recorder()
    if args.mode == "public":
        public(recorder, cfg, args.date)
    elif args.mode == "gtfs":
        gtfs(recorder, cfg)
    elif args.mode == "flight":
        recorder.capture("flight", "vehicle-points", cfg["flight"])
    else:
        verify()
