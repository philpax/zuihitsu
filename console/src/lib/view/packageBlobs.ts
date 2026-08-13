import { useEffect, useState } from "react";

import type { PackageBlob } from "@zuihitsu/wire/types/PackageBlob.ts";
import { servedMediaType, whenWasmReady } from "../replica/replica.ts";
import { NO_BLOBS, type BlobSource } from "./blobSource.ts";

/// Resolve a package's attachments against the blob catalogue it carries, minting one object URL per
/// blob — the eval frame's [`BlobSource`], where no agent stands behind the viewer to serve bytes.
///
/// Object URLs rather than `data:` URIs deliberately. A browser blocks a top-level navigation to a
/// `data:` URL, so the transcript's "open the whole file" link and the download chip would quietly do
/// nothing; a `blob:` URL navigates, downloads, and renders in an `<img>` exactly as a served one does,
/// which is what keeps the eval and live frames on one rendering path.
///
/// The URLs are revoked when the catalogue changes or the frame unmounts. An address the catalogue
/// does not carry resolves to `null`, and the attachment degrades to its announcement.
/// A package recorded before the catalogue existed has no field at all, so the hook takes it as
/// optional rather than making every caller spell the fallback — and a shared empty constant keeps the
/// effect's dependency stable, where a fresh `[]` per render would re-mint every URL each time.
export function usePackageBlobs(blobs: readonly PackageBlob[] | undefined): BlobSource {
  const catalogue = blobs ?? NO_CATALOGUE;
  // Keyed by the catalogue it was minted from and matched during render, the shape `useReplica` and
  // `useRunRecord` use: a catalogue change reads as unresolved immediately, without a state write in
  // the effect, and only the async minting writes state.
  const [minted, setMinted] = useState<{
    catalogue: readonly PackageBlob[];
    urls: ReadonlyMap<string, string>;
  } | null>(null);

  useEffect(() => {
    if (catalogue.length === 0) return;
    let revoked = false;
    let urls: string[] = [];
    // The media type comes from Rust, so the effect waits for the wasm module: the eval frame renders
    // before any run is folded, and a `Replica` is what usually guarantees it is loaded.
    void whenWasmReady().then(() => {
      if (revoked) return;
      const map = new Map<string, string>();
      for (const blob of catalogue) {
        const url = URL.createObjectURL(objectBlob(blob));
        urls.push(url);
        map.set(blob.hash, url);
      }
      setMinted({ catalogue, urls: map });
    });
    return () => {
      revoked = true;
      for (const url of urls) URL.revokeObjectURL(url);
      urls = [];
    };
  }, [catalogue]);

  if (minted === null || minted.catalogue !== catalogue) return NO_BLOBS;
  return { urlFor: ({ blob }) => minted.urls.get(blob) ?? null };
}

const NO_CATALOGUE: readonly PackageBlob[] = [];

/// One catalogue entry as a browser `Blob`, typed by what the agent's read route would serve it as
/// rather than by what it was stored as — markup a run shared is read as text here too, not run.
export function objectBlob(blob: PackageBlob): Blob {
  return new Blob([decodeBase64(blob.base64)], { type: servedMediaType(blob.mime) });
}

/// Decode standard-alphabet base64 to bytes. `atob` yields one character per byte, so the copy is the
/// decode: each character's code unit is the byte itself. The buffer is allocated explicitly because a
/// `Blob` part must be backed by an `ArrayBuffer` rather than by any `ArrayBufferLike`.
export function decodeBase64(base64: string): Uint8Array<ArrayBuffer> {
  const binary = atob(base64);
  const bytes = new Uint8Array(new ArrayBuffer(binary.length));
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}
