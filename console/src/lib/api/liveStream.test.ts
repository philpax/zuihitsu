import { describe, expect, it } from "vitest";

import { SseDecoder } from "./liveStream.ts";

const encode = (text: string) => new TextEncoder().encode(text);

describe("SseDecoder", () => {
  it("parses frames and buffers partials across chunk boundaries", () => {
    const decoder = new SseDecoder();
    // The frame boundary lands mid-frame: nothing complete yet.
    expect(decoder.push(encode('data: {"type":"event","seq'))).toEqual([]);
    // The rest of the frame plus the start of another.
    const frames = decoder.push(encode('":1}\n\ndata: {"type":"progress"'));
    expect(frames).toEqual([{ kind: "event", data: { type: "event", seq: 1 } }]);
    expect(decoder.push(encode("}\n\n"))).toEqual([
      { kind: "progress", data: { type: "progress" } },
    ]);
  });

  it("tolerates CRLF line endings", () => {
    const decoder = new SseDecoder();
    expect(decoder.push(encode('data: {"type":"event","seq":1}\r\n\r\n'))).toEqual([
      { kind: "event", data: { type: "event", seq: 1 } },
    ]);
  });

  it("swallows keep-alive comments", () => {
    const decoder = new SseDecoder();
    const frames = decoder.push(encode(':keep-alive\n\ndata: {"type":"event","seq":1}\n\n'));
    expect(frames).toEqual([{ kind: "event", data: { type: "event", seq: 1 } }]);
  });
});
