import { useRef, useState } from "react";

import type { BlobHash } from "@zuihitsu/wire/types/BlobHash.ts";
import { Button, Hint } from "../../components/primitives.tsx";
import { PendingAttachmentChips } from "./Attachments.tsx";
import {
  type PendingAttachment,
  attachmentsHoldSend,
  outboundAttachments,
  pendingAttachment,
  withStatus,
  type SentAttachment,
} from "./attachmentUtilities.ts";

/// A message composer: a growing input that sends on Enter (Shift+Enter for a newline), with a
/// pending state while the turn runs and any failure surfaced inline. `onSend` runs the turn — the
/// caller chooses the endpoint and authority (a participant message or an operator imprint) — and
/// the reply arrives through the live tail. `onPendingChange` lets the conversation show that the
/// agent is working while the turn is in flight.
///
/// Files ride along when the caller supplies `onUpload`: picked, pasted, or dropped, each goes up as
/// it is added, so the send names blobs the agent already holds and a failed upload surfaces while
/// the sender is still typing. Without `onUpload` the composer has no file affordances at all — the
/// operator's imprint channel takes none, an imprint being a note rather than a message.
export function Composer({
  onSend,
  onUpload,
  onPendingChange,
  placeholder = "Write to the agent…",
  disabled = false,
  disabledHint,
}: {
  onSend: (text: string, attachments: SentAttachment[]) => Promise<void>;
  onUpload?: (file: File) => Promise<BlobHash>;
  onPendingChange?: (pending: boolean) => void;
  placeholder?: string;
  disabled?: boolean;
  disabledHint?: string;
}) {
  const [draft, setDraft] = useState("");
  const [attachments, setAttachments] = useState<PendingAttachment[]>([]);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // True while a drag hovers the composer, so the writing line shows it will take the drop.
  const [dragging, setDragging] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  const attaching = onUpload !== undefined && !disabled && !pending;
  const held = attachmentsHoldSend(attachments);
  const empty = draft.trim().length === 0 && attachments.length === 0;

  async function upload(attachment: PendingAttachment) {
    if (!onUpload) return;
    try {
      const blob = await onUpload(attachment.file);
      setAttachments((prev) => withStatus(prev, attachment.id, { state: "ready", blob }));
    } catch (cause) {
      const message = cause instanceof Error ? cause.message : String(cause);
      setAttachments((prev) =>
        withStatus(prev, attachment.id, { state: "failed", error: message }),
      );
    }
  }

  function addFiles(files: FileList | null) {
    if (!attaching || !files || files.length === 0) return;
    for (const file of Array.from(files)) {
      const attachment = pendingAttachment(file);
      setAttachments((prev) => [...prev, attachment]);
      void upload(attachment);
    }
  }

  function retry(id: string) {
    const attachment = attachments.find((candidate) => candidate.id === id);
    if (!attachment) return;
    setAttachments((prev) => withStatus(prev, id, { state: "uploading" }));
    void upload(attachment);
  }

  async function send() {
    const text = draft.trim();
    // An attachment alone is a message — an image is often the whole of what someone has to say — so
    // the send needs text or files, not text. It holds while any file is unsettled: see
    // `attachmentsHoldSend`.
    if (empty || held || pending || disabled) return;
    const carried = outboundAttachments(attachments);
    // Clear the box at once, so it does not sit showing the sent text while the agent works; a failed
    // send restores it below, so nothing is lost.
    setDraft("");
    setAttachments([]);
    setError(null);
    setPending(true);
    onPendingChange?.(true);
    try {
      await onSend(text, carried);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      setDraft(text);
      // The blobs are still stored — the address is the content — so the files come back attached and
      // ready rather than needing a re-upload.
      setAttachments(attachments);
    } finally {
      setPending(false);
      onPendingChange?.(false);
    }
  }

  // The textarea and an invisible twin share one grid cell: the twin renders the draft as wrapped
  // text, so the cell — and with it the textarea — grows line by line as you type, capped before it
  // eats the transcript. Robust everywhere, unlike `field-sizing: content`.
  const grown =
    "col-start-1 row-start-1 max-h-44 px-3.5 py-2.5 text-base leading-relaxed break-words";
  const failed = attachments.find((attachment) => attachment.status.state === "failed");
  return (
    <div
      onDragOver={(event) => {
        if (!attaching) return;
        event.preventDefault();
        setDragging(true);
      }}
      onDragLeave={() => setDragging(false)}
      onDrop={(event) => {
        if (!attaching) return;
        event.preventDefault();
        setDragging(false);
        addFiles(event.dataTransfer.files);
      }}
    >
      {attachments.length > 0 && (
        <div className="mb-1.5">
          <PendingAttachmentChips
            attachments={attachments}
            onRemove={(id) =>
              setAttachments((prev) => prev.filter((attachment) => attachment.id !== id))
            }
            onRetry={retry}
          />
        </div>
      )}
      <div
        className={
          "flex items-center rounded-sm border bg-paper-raised transition-colors focus-within:border-line-strong " +
          (dragging ? "border-clay" : "border-line")
        }
      >
        <div className="grid min-w-0 flex-1">
          <div aria-hidden className={`${grown} invisible overflow-hidden whitespace-pre-wrap`}>
            {draft + " "}
          </div>
          <textarea
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onPaste={(event) => {
              // A pasted image arrives as a clipboard file with no text beside it; take it as an
              // attachment and let anything else paste as the text it is.
              if (!attaching || event.clipboardData.files.length === 0) return;
              event.preventDefault();
              addFiles(event.clipboardData.files);
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !event.shiftKey) {
                event.preventDefault();
                send();
              }
            }}
            rows={1}
            placeholder={
              disabled ? disabledHint : pending ? "Waiting for the agent's reply…" : placeholder
            }
            disabled={pending || disabled}
            className={`${grown} w-full resize-none overflow-y-auto bg-transparent text-ink placeholder:text-ink-faint/60 focus:outline-none disabled:opacity-60`}
          />
        </div>
        {onUpload && (
          <>
            <input
              ref={fileRef}
              type="file"
              multiple
              className="hidden"
              onChange={(event) => {
                addFiles(event.target.files);
                // Clear the picker, so picking the same file again is a second attachment rather than
                // a change event that never fires.
                event.target.value = "";
              }}
            />
            <button
              onClick={() => fileRef.current?.click()}
              disabled={!attaching}
              title="Attach a file — you can also paste or drop one"
              className="shrink-0 px-2 font-mono text-xs text-ink-faint transition-colors hover:text-clay disabled:opacity-45 disabled:hover:text-ink-faint"
            >
              attach
            </button>
          </>
        )}
        <Button
          primary
          className="mx-2 shrink-0"
          onClick={send}
          disabled={pending || disabled || empty || held}
        >
          {pending ? "…" : "send"}
        </Button>
      </div>
      <div className="mt-1.5 flex min-h-4 items-baseline">
        {error ? (
          <Hint tone="error">{error}</Hint>
        ) : failed && failed.status.state === "failed" ? (
          <Hint tone="error">
            {failed.name} did not upload — {failed.status.error}. Retry it or remove it to send.
          </Hint>
        ) : held ? (
          <Hint className="text-2xs">attaching — the send waits for the files</Hint>
        ) : (
          <Hint className="hidden text-2xs sm:inline">
            enter to send · shift+enter for a newline
            {onUpload && " · paste or drop a file to attach it"}
          </Hint>
        )}
      </div>
    </div>
  );
}
