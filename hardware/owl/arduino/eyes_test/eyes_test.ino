/* Two-eye animation test for 2x GC9A01 1.28" round displays on Arduino Uno.
 * Port of eyes_test.py (same look: sclera, blue iris, pupil, glint, blinking lids).
 *
 * The Uno has 2 KB of RAM, so there is no framebuffer. Each frame only the
 * rectangle that changed (iris movement, lid band) is re-rendered one row at a
 * time into a 480-byte line buffer and pushed to both displays -> no flicker.
 *
 * Library: "Adafruit GC9A01A" (Library Manager; also installs Adafruit GFX).
 *
 * Wiring (shared hardware SPI bus, separate CS / DC / RST per eye):
 *   Display pin | Left eye | Right eye
 *   VCC         | 3.3V     | 3.3V
 *   GND         | GND      | GND
 *   SCL         | D13      | D13
 *   SDA         | D11      | D11
 *   CS          | D10      | D7
 *   DC          | D9       | D6
 *   RST         | D8       | D5
 *   BL (if any) | 3.3V     | 3.3V
 *
 * !! Uno pins are 5V. Most GC9A01 modules are 3.3V logic. Unless your board
 *    says it is 5V tolerant, put a level shifter (e.g. 74HC4050) or resistor
 *    dividers (1k/2k) on SCL, SDA, CS, DC, RST.
 *
 * Set NUM_EYES to 1 for a first wiring check (left eye only).
 * Serial monitor (115200) prints fps every 5 s.
 */
#include <SPI.h>
#include <Adafruit_GFX.h>
#include <Adafruit_GC9A01A.h>

#define NUM_EYES 2

#define RGB565(r, g, b) (uint16_t)((((r) & 0xF8) << 8) | (((g) & 0xFC) << 3) | ((b) >> 3))

const int SIZE = 240;
const int C = SIZE / 2;
const int SCLERA_R = 116;
const int IRIS_R = 62;
const int PUPIL_R = 28;
const int GLINT_R = 9;
const int GLINT_DX = -21;  // glint centre relative to iris centre
const int GLINT_DY = -29;
const unsigned long BLINK_MS = 300;

const uint16_t COL_BG     = RGB565(0, 0, 0);
const uint16_t COL_SCLERA = RGB565(235, 235, 235);
const uint16_t COL_IRIS   = RGB565(30, 120, 200);
const uint16_t COL_PUPIL  = RGB565(0, 0, 0);
const uint16_t COL_GLINT  = RGB565(255, 255, 255);
const uint16_t COL_LID    = RGB565(20, 20, 20);

Adafruit_GC9A01A eyes[] = {
  Adafruit_GC9A01A(10, 9, 8),  // left:  CS, DC, RST
  Adafruit_GC9A01A(7, 6, 5),   // right: CS, DC, RST
};

// Half-width of each circle at vertical distance d from its centre.
uint8_t hwSclera[SCLERA_R + 1], hwIris[IRIS_R + 1], hwPupil[PUPIL_R + 1], hwGlint[GLINT_R + 1];
uint16_t rowBuf[SIZE];

// Current state used by render().
int irisX = C, irisY = C;  // iris centre in pixels
int lidH = 0;              // rows covered by each lid, 0 = open, C = closed

void buildTable(uint8_t *t, int r) {
  for (int d = 0; d <= r; d++)
    t[d] = (uint8_t)sqrt((float)r * r - (float)d * d);
}

void span(int cx, int hw, uint16_t col, int x0, int x1) {
  int a = cx - hw, b = cx + hw;
  if (a < x0) a = x0;
  if (b > x1) b = x1;
  for (int x = a; x <= b; x++) rowBuf[x - x0] = col;
}

void disc(int y, int cx, int cy, int r, const uint8_t *tbl, uint16_t col, int x0, int x1) {
  int d = abs(y - cy);
  if (d <= r) span(cx, tbl[d], col, x0, x1);
}

// Redraw the inclusive rectangle (x0,y0)-(x1,y1) on every eye.
void render(int x0, int y0, int x1, int y1) {
  x0 = max(x0, 0); y0 = max(y0, 0);
  x1 = min(x1, SIZE - 1); y1 = min(y1, SIZE - 1);
  if (x0 > x1 || y0 > y1) return;
  int w = x1 - x0 + 1;

  for (int y = y0; y <= y1; y++) {
    if (y < lidH || y >= SIZE - lidH) {
      for (int i = 0; i < w; i++) rowBuf[i] = COL_LID;
    } else {
      for (int i = 0; i < w; i++) rowBuf[i] = COL_BG;
      disc(y, C, C, SCLERA_R, hwSclera, COL_SCLERA, x0, x1);
      disc(y, irisX, irisY, IRIS_R, hwIris, COL_IRIS, x0, x1);
      disc(y, irisX, irisY, PUPIL_R, hwPupil, COL_PUPIL, x0, x1);
      disc(y, irisX + GLINT_DX, irisY + GLINT_DY, GLINT_R, hwGlint, COL_GLINT, x0, x1);
    }
    for (int e = 0; e < NUM_EYES; e++) {
      eyes[e].startWrite();
      eyes[e].setAddrWindow(x0, y, w, 1);
      eyes[e].writePixels(rowBuf, w);
      eyes[e].endWrite();
    }
  }
}

float px = 0, py = 0, tx = 0, ty = 0;
unsigned long nextLook, nextBlink, blinkStart;
bool blinking = false;
unsigned long frames = 0, t0;

void setup() {
  Serial.begin(115200);
  randomSeed(analogRead(A0));
  buildTable(hwSclera, SCLERA_R);
  buildTable(hwIris, IRIS_R);
  buildTable(hwPupil, PUPIL_R);
  buildTable(hwGlint, GLINT_R);

  for (int e = 0; e < NUM_EYES; e++) eyes[e].begin();
  render(0, 0, SIZE - 1, SIZE - 1);

  unsigned long now = millis();
  nextLook = now;
  nextBlink = now + random(2000, 5000);
  t0 = now;
}

void loop() {
  unsigned long now = millis();

  // Saccades: pick a new gaze target every 0.6-2.5 s, ease toward it.
  if (now >= nextLook) {
    tx = random(-45, 46);
    ty = random(-35, 36);
    nextLook = now + random(600, 2500);
  }
  px += (tx - px) * 0.25;
  py += (ty - py) * 0.25;

  // Blink: lid closes then opens over BLINK_MS, every 2-6 s.
  float lid = 0;
  if (!blinking && now >= nextBlink) {
    blinking = true;
    blinkStart = now;
  }
  if (blinking) {
    float t = (now - blinkStart) / (float)BLINK_MS;
    if (t >= 1) {
      blinking = false;
      nextBlink = now + random(2000, 6000);
    } else {
      lid = 1 - fabs(2 * t - 1);
    }
  }

  int newLid = (int)(C * lid);
  int newX = C + (int)px, newY = C + (int)py;
  int oldLid = lidH, oldX = irisX, oldY = irisY;
  lidH = newLid;
  irisX = newX;
  irisY = newY;

  // Only redraw what changed.
  if (newLid != oldLid) {
    int a = min(oldLid, newLid), b = max(oldLid, newLid);
    render(0, a, SIZE - 1, b - 1);                // top lid band
    render(0, SIZE - b, SIZE - 1, SIZE - a - 1);  // bottom lid band
  }
  if (newX != oldX || newY != oldY) {
    render(min(oldX, newX) - IRIS_R, min(oldY, newY) - IRIS_R,
           max(oldX, newX) + IRIS_R, max(oldY, newY) + IRIS_R);
  }

  frames++;
  if (now - t0 >= 5000) {
    Serial.print(frames * 1000.0 / (now - t0), 1);
    Serial.println(" fps");
    frames = 0;
    t0 = now;
  }
}
