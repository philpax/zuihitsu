import { describe, expect, it } from "vitest";

import type { PackageBlob } from "@zuihitsu/wire/types/PackageBlob.ts";
import { decodeBase64, objectBlob } from "./packageBlobs.ts";

/// The PNG signature, base64-encoded — a stand-in for a fixture's bytes, chosen because a wrong decode
/// is obvious rather than plausible.
const PNG_SIGNATURE = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

function blob(mime: string, bytes: Uint8Array): PackageBlob {
  return {
    hash: "0".repeat(64),
    mime,
    base64: btoa(String.fromCharCode(...bytes)),
  };
}

describe("a package's blob catalogue", () => {
  it("decodes base64 to the exact bytes, high bytes included", () => {
    // A byte above 0x7f is where a charCode-vs-codePoint slip would show, so the signature's 0x89
    // leads.
    expect([...decodeBase64(blob("image/png", PNG_SIGNATURE).base64)]).toEqual([...PNG_SIGNATURE]);
    expect(decodeBase64("")).toHaveLength(0);
  });

  it("types the object blob as the agent would serve it, not as it was stored", async () => {
    // The eval frame mints its own URLs on the console's own origin, so markup a run shared must be
    // presented as text here exactly as the agent's read route presents it.
    const markup = new TextEncoder().encode("<html><script>alert(1)</script></html>");
    const shared = objectBlob(blob("text/html", markup));
    expect(shared.type).toBe("text/plain; charset=utf-8");
    // The bytes themselves are untouched — only the declared type differs.
    expect(await shared.text()).toBe("<html><script>alert(1)</script></html>");

    // A perceivable image stays itself, or the transcript has nothing to put in an `<img>`.
    expect(objectBlob(blob("image/png", PNG_SIGNATURE)).type).toBe("image/png");
  });
});
