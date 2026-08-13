// @vitest-environment jsdom
import { beforeAll, describe, expect, it } from "vitest";
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";

import type { Attachment } from "@zuihitsu/wire/types/Attachment.ts";
import { BlobSourceContext, NO_BLOBS, servedBlobs } from "../../lib/view/blobSource.ts";
import { AttachmentStrip } from "./Attachments.tsx";

// The strip is the one view that renders bytes, and the two frames differ only in the source they
// provide — a served agent, or an eval package's own catalogue. What is pinned here is that the strip
// reads its source rather than knowing which frame it is in: a resolving source renders the image, and
// a source that reaches nothing degrades to the announcement instead of a broken image.

beforeAll(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
});

function attachment(overrides: Partial<Attachment> = {}): Attachment {
  return {
    name: "cover-draft.png",
    mime: "image/png",
    blob: "a".repeat(64),
    byte_len: 1195,
    kind: "Image",
    ...overrides,
  };
}

function render(source: Parameters<typeof BlobSourceContext.Provider>[0]["value"]) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => {
    root.render(
      <StrictMode>
        <BlobSourceContext.Provider value={source}>
          <AttachmentStrip
            attachments={[
              attachment(),
              attachment({
                name: "venue.txt",
                mime: "text/plain",
                kind: "Text",
                blob: "b".repeat(64),
              }),
            ]}
          />
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

describe("the attachment strip", () => {
  it("renders the image and opens the text file when its source resolves", () => {
    // An object URL from an eval package and a served URL are the same thing to the strip; a served
    // base is used here because it is the readable one to assert against.
    const { container, unmount } = render(servedBlobs(""));

    const image = container.querySelector("img");
    expect(image?.getAttribute("src")).toBe(`/blobs/${"a".repeat(64)}`);
    expect(image?.getAttribute("alt")).toBe("cover-draft.png");
    // The text file is disclosed rather than shown, so its excerpt is fetched only when opened.
    expect(container.textContent).toContain("venue.txt");
    expect(container.querySelector("button")).toBeTruthy();

    unmount();
  });

  it("announces every attachment when nothing can reach the bytes", () => {
    const { container, unmount } = render(NO_BLOBS);

    expect(container.querySelector("img")).toBeNull();
    // The record is all it has, and it states all of it rather than rendering a broken image.
    expect(container.textContent).toContain("cover-draft.png");
    expect(container.textContent).toContain("image/png");
    expect(container.textContent).toContain("1.2 KB");

    unmount();
  });
});
