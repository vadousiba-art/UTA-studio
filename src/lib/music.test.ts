import { describe, expect, it } from "vitest";
import { centsBetween, formatCents, formatDb, formatTime, hzToMidi, midiToHz, noteName, noteWithCents } from "./music";

describe("music helpers", () => {
  it("converts pitch", () => {
    expect(midiToHz(69)).toBeCloseTo(440);
    expect(hzToMidi(220)).toBeCloseTo(57);
    expect(centsBetween(445, 440)).toBeCloseTo(19.56, 1);
    expect(noteName(60)).toBe("C4");
    expect(noteName(57.4)).toBe("A3");
    expect(noteWithCents(56.88)).toEqual({ name: "A3", cents: -12 });
  });
  it("formats values", () => {
    expect(formatCents(12.4)).toBe("+12¢");
    expect(formatCents(-3)).toBe("−3¢");
    expect(formatCents(0.2)).toBe("±0¢");
    expect(formatTime(75.4)).toBe("1:15");
    expect(formatTime(5.256, true)).toBe("0:05.26");
    expect(formatDb(-120)).toBe("−∞");
    expect(formatDb(3)).toBe("+3.0");
  });
});
