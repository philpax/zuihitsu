// @vitest-environment jsdom
import { beforeAll, describe, expect, it } from "vitest";
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { BlobSourceContext, NO_BLOBS, servedBlobs } from "../lib/view/blobSource.ts";
import { BlobRef } from "./eventDetailParts.tsx";

// The reference family's rule: every raw id an event payload carries links to where that thing
// lives, and degrades to plain text rather than to a link that goes nowhere. A content address is
// such an id.

const ADDRESS = "a".repeat(64);

beforeAll(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
});

function render(source: Parameters<typeof BlobSourceContext.Provider>[0]["value"]) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => {
    root.render(
      <StrictMode>
        <BlobSourceContext.Provider value={source}>
          <BlobRef blob={ADDRESS} name="cover-draft.png" />
        </BlobSourceContext.Provider>
      </StrictMode>,
    );
  });
  return {
    container,
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

describe("an attachment reference in an event detail", () => {
  it("links the file to its bytes and shows the head of the address", () => {
    const { container, unmount } = render(servedBlobs(""));

    const link = container.querySelector("a");
    expect(link?.getAttribute("href")).toBe(`/blobs/${ADDRESS}`);
    expect(link?.textContent).toBe("cover-draft.png");
    // The whole address stays reachable for copying, without 64 characters of hex on the line.
    expect(link?.getAttribute("title")).toContain(ADDRESS);
    expect(container.textContent).toContain("aaaaaaaaaaaa…");
    expect(container.textContent).not.toContain(ADDRESS);

    unmount();
  });

  it("degrades to plain text when nothing can reach the bytes", () => {
    const { container, unmount } = render(NO_BLOBS);

    expect(container.querySelector("a")).toBeNull();
    // Still names the file and its address — the record is legible either way.
    expect(container.textContent).toContain("cover-draft.png");
    expect(container.textContent).toContain("aaaaaaaaaaaa…");

    unmount();
  });
});
