import { describe, expect, it } from "vitest";

import { SseDecoder } from "./liveStream.ts";

const encode = (text: string) => new TextEncoder().encode(text);

describe("SseDecoder", () => {
  it("parses frames and buffers partials across chunk boundaries", () => {
    const decoder = new SseDecoder();
    // The frame boundary lands mid-frame: nothing complete yet.
    expect(decoder.push(encode('event: event\ndata: {"seq'))).toEqual([]);
    // The rest of the frame plus the start of another.
    const frames = decoder.push(encode('":1}\n\nevent: progress\ndata: {"kind"'));
    expect(frames).toEqual([{ kind: "event", data: '{"seq":1}' }]);
    expect(decoder.push(encode(':"reply"}\n\n'))).toEqual([
      { kind: "progress", data: '{"kind":"reply"}' },
    ]);
  });

  it("tolerates CRLF line endings", () => {
    const decoder = new SseDecoder();
    expect(decoder.push(encode("event: event\r\ndata: 1\r\n\r\n"))).toEqual([
      { kind: "event", data: "1" },
    ]);
  });

  it("joins multi-line data and swallows keep-alive comments", () => {
    const decoder = new SseDecoder();
    const frames = decoder.push(encode(":keep-alive\n\nevent: event\ndata: a\ndata: b\n\n"));
    expect(frames).toEqual([{ kind: "event", data: "a\nb" }]);
  });
});
