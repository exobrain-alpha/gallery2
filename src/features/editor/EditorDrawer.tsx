import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
  type MouseEvent,
} from "react";
import { Icons } from "../../icons";
import type { Attachment, ImageRecord, PickedImage, XaiEditResult, XaiKeyStatus } from "../../types";
import { classNames, formatErrorMessage, mediaName, rawErrorMessage } from "../../utils";

const MAX_REFERENCES = 3;
const DRAWER_ANIMATION_MS = 420;
const COPY_FEEDBACK_MS = 1100;
const RETRY_FEEDBACK_MS = 520;
const NOTICE_OK_MS = 1600;
const IMAGE_COUNT_VALUES = [1, 2, 3, 4];
const SESSION_SAVE_DEBOUNCE_MS = 260;
const BOTTOM_STICK_THRESHOLD = 80;

const ASPECT_BUTTONS = [
  { key: "auto", kind: "single", values: ["auto"], label: "Auto" },
  { key: "1:1", kind: "single", values: ["1:1"], label: "1:1" },
  { key: "16-9", kind: "pair", values: ["16:9", "9:16"], label: "16:9" },
  { key: "4-3", kind: "pair", values: ["4:3", "3:4"], label: "4:3" },
  { key: "3-2", kind: "pair", values: ["3:2", "2:3"], label: "3:2" },
  { key: "2-1", kind: "pair", values: ["2:1", "1:2"], label: "2:1" },
  { key: "20-9", kind: "pair", values: ["20:9", "9:20"], label: "20:9" },
] as const;

const RESOLUTION_BUTTONS = [
  { value: "1k", label: "1K" },
  { value: "2k", label: "2K" },
] as const;

type Role = "user" | "assistant";

interface Message {
  id: string;
  role: Role;
  content: string;
  attachments: Attachment[];
  pending?: boolean;
  tone?: "error" | "";
  aspectRatio?: string | null;
  resolution?: string | null;
  imageCount?: number;
}

interface EditorSessionState {
  sessionId: string;
  messages: Message[];
}

interface RunPromptOptions {
  prompt: string;
  attachments: Attachment[];
  aspectRatio: string;
  resolution: string | null;
  imageCount: number;
  clearComposer: boolean;
}

interface EditorNoticeState {
  message: string;
  tone: "ok" | "error" | "";
}

export interface EditorDrawerHandle {
  open: (record: ImageRecord | ImageRecord[]) => Promise<void>;
  close: () => void;
  isOpen: () => boolean;
}

interface EditorDrawerProps {
  readImageDataUri: (path: string) => Promise<string>;
  getXaiKeyStatus: () => Promise<XaiKeyStatus>;
  pickReferenceImages: () => Promise<PickedImage[]>;
  editImage: (payload: {
    sourcePaths: string[];
    sourceDataUris: string[];
    prompt: string;
    aspectRatio: string | null;
    resolution: string | null;
    imageCount: number;
  }) => Promise<XaiEditResult>;
  onPreviewAttachment: (attachment: Attachment) => void;
  onToggle: (open: boolean) => void;
  onError: (error: unknown, label?: string) => void;
}

export const EditorDrawer = forwardRef<EditorDrawerHandle, EditorDrawerProps>(function EditorDrawer(
  { readImageDataUri, getXaiKeyStatus, pickReferenceImages, editImage, onPreviewAttachment, onToggle, onError },
  ref,
) {
  const [open, setOpen] = useState(false);
  const [closing, setClosing] = useState(false);
  const [pending, setPending] = useState(false);
  const [aspectRatio, setAspectRatio] = useState("auto");
  const [resolution, setResolution] = useState<string | null>(null);
  const [imageCount, setImageCount] = useState(1);
  const [composerText, setComposerText] = useState("");
  const [selectedAttachments, setSelectedAttachments] = useState<Attachment[]>([]);
  const [messages, setMessages] = useState<Message[]>([]);
  const [notice, setNotice] = useState<EditorNoticeState>({ message: "", tone: "" });
  const [xaiKeyConfigured, setXaiKeyConfigured] = useState<boolean | null>(null);
  const [actionFeedback, setActionFeedback] = useState<Record<string, string>>({});
  const [sessionId, setSessionId] = useState("");
  const [sessionLoaded, setSessionLoaded] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const messagesRef = useRef<HTMLDivElement | null>(null);
  const closeTimerRef = useRef<number>(0);
  const focusTimerRef = useRef<number>(0);
  const saveTimerRef = useRef<number>(0);
  const noticeTimerRef = useRef<number>(0);
  const sessionTokenRef = useRef(0);
  const messagesStateRef = useRef<Message[]>([]);
  const shouldStickToBottomRef = useRef(true);

  useEffect(() => {
    loadCurrentSession().catch((error) => onError(error, "加载会话失败"));
  }, []);

  useEffect(() => {
    messagesStateRef.current = messages;
    const messagesElement = messagesRef.current;
    if (!messagesElement || !shouldStickToBottomRef.current) return;
    window.requestAnimationFrame(() => scrollMessagesToBottom(messagesElement));
  }, [messages]);

  useEffect(() => {
    if (!sessionLoaded || !sessionId) return;
    if (saveTimerRef.current) window.clearTimeout(saveTimerRef.current);
    saveTimerRef.current = window.setTimeout(() => {
      persistSession(sessionId, messagesStateRef.current).catch((error) => onError(error, "保存会话失败"));
    }, SESSION_SAVE_DEBOUNCE_MS);
  }, [messages, sessionId, sessionLoaded]);

  useEffect(() => {
    if (open && !pending) focusComposer();
  }, [open, pending]);

  useEffect(() => {
    resizeTextarea(inputRef.current);
  }, [composerText]);

  useEffect(() => {
    return () => {
      if (closeTimerRef.current) window.clearTimeout(closeTimerRef.current);
      if (focusTimerRef.current) window.clearTimeout(focusTimerRef.current);
      if (saveTimerRef.current) window.clearTimeout(saveTimerRef.current);
      if (noticeTimerRef.current) window.clearTimeout(noticeTimerRef.current);
    };
  }, []);

  useImperativeHandle(ref, () => ({
    open: openDrawer,
    close: closeDrawer,
    isOpen: () => open,
  }), [open, sessionLoaded]);

  async function openDrawer(record: ImageRecord | ImageRecord[]) {
    const records = (Array.isArray(record) ? record : [record])
      .filter((item) => item?.mediaType === "image")
      .slice(0, MAX_REFERENCES);
    if (!records.length) return;
    if (closeTimerRef.current) window.clearTimeout(closeTimerRef.current);
    const token = nextSessionToken();
    const sourceAttachments = records.map((item) => createAttachment({
      path: item.path,
      dataUrl: convertFileSrc(item.path),
    }));
    const attachments = sourceAttachments.filter(Boolean) as Attachment[];
    if (!attachments.length) return;
    if (!sessionLoaded) {
      await loadCurrentSession();
      if (token !== sessionTokenRef.current) return;
    }

    setOpen(false);
    setClosing(false);
    setAspectRatio("auto");
    setResolution(null);
    setImageCount(1);
    setComposerText("");
    setSelectedAttachments(attachments);
    setNotice({ message: "", tone: "" });
    setXaiKeyConfigured(null);
    setActionFeedback({});
    window.requestAnimationFrame(() => {
      setOpen(true);
      onToggle(true);
      focusComposer();
      checkXaiKeyStatusForNotice().catch((error) => {
        showNotice(formatErrorMessage(error, "API Key 检查失败"), "error");
        onError(error, "API Key 检查失败");
      });
    });
  }

  function closeDrawer() {
    if (!open && !closing) return;
    setOpen(false);
    setClosing(true);
    onToggle(false);
    closeTimerRef.current = window.setTimeout(() => {
      setClosing(false);
      setAspectRatio("auto");
      setResolution(null);
      setImageCount(1);
      setComposerText("");
      setSelectedAttachments([]);
      setNotice({ message: "", tone: "" });
      setXaiKeyConfigured(null);
      setActionFeedback({});
    }, DRAWER_ANIMATION_MS);
  }

  async function loadCurrentSession() {
    const state = await invoke<EditorSessionState>("load_editor_session");
    setSessionId(state.sessionId);
    setMessages(normalizePersistedMessages(state.messages));
    setSessionLoaded(true);
  }

  function nextSessionToken() {
    sessionTokenRef.current += 1;
    return sessionTokenRef.current;
  }

  function focusComposer() {
    if (focusTimerRef.current) window.clearTimeout(focusTimerRef.current);
    focusTimerRef.current = window.setTimeout(() => {
      const input = inputRef.current;
      if (!input || input.disabled) return;
      input.focus({ preventScroll: true });
      input.setSelectionRange(input.value.length, input.value.length);
    }, 0);
  }

  function handleMessagesScroll() {
    const messagesElement = messagesRef.current;
    if (!messagesElement) return;
    shouldStickToBottomRef.current = isNearMessagesBottom(messagesElement);
  }

  function applyAspectToggle(key: string | undefined) {
    const nextRatio = nextAspectRatio(key, aspectRatio);
    if (nextRatio) setAspectRatio(nextRatio);
  }

  function toggleResolution(value: string) {
    setResolution((current) => (current === value ? null : value));
  }

  function cycleImageCount() {
    setImageCount((current) => nextImageCount(current));
  }

  async function handlePickReferences() {
    if (pending) return;
    const picked = await pickReferenceImages();
    const items = normalizePickedReferences(picked);
    if (!items.length) return;
    setSelectedAttachments((current) => dedupeAttachments([...current, ...items]));
  }

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    submitPrompt().catch((error) => onError(error, "生成失败"));
  }

  function handleKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === "Escape") {
      if (document.body.classList.contains("previewing")) return;
      event.stopPropagation();
      closeDrawer();
      return;
    }
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      submitPrompt().catch((error) => onError(error, "生成失败"));
    }
  }

  function submitPrompt() {
    const prompt = composerText.trim();
    if (!open || pending || !prompt || !selectedAttachments.length) return Promise.resolve();
    return ensureXaiKeyReady().then((ready) => {
      if (!ready) return;
      return runPrompt({
        prompt,
        attachments: selectedAttachments,
        aspectRatio,
        resolution,
        imageCount,
        clearComposer: true,
      });
    });
  }

  async function ensureXaiKeyReady() {
    if (xaiKeyConfigured === true) return true;
    try {
      const status = await checkXaiKeyStatusForNotice();
      return status.configured;
    } catch (error) {
      showNotice(formatErrorMessage(error, "API Key 检查失败"), "error");
      throw error;
    }
  }

  async function checkXaiKeyStatusForNotice() {
    const status = await getXaiKeyStatus();
    setXaiKeyConfigured(status.configured);
    if (status.configured) {
      showNotice("API Key 已就绪", "ok", NOTICE_OK_MS);
    } else {
      showNotice("请先设置 xAI API Key", "error");
    }
    return status;
  }

  function showNotice(message: string, tone: EditorNoticeState["tone"], duration = 0) {
    if (noticeTimerRef.current) window.clearTimeout(noticeTimerRef.current);
    setNotice({ message, tone });
    if (duration > 0) {
      noticeTimerRef.current = window.setTimeout(() => {
        setNotice((current) => (current.message === message ? { message: "", tone: "" } : current));
      }, duration);
    }
  }

  function clearNotice() {
    if (noticeTimerRef.current) window.clearTimeout(noticeTimerRef.current);
    setNotice({ message: "", tone: "" });
  }

  function notifyXaiKeyError(error: unknown) {
    const message = formatErrorMessage(error, "生成失败");
    if (isXaiKeyStatusError(error)) {
      setXaiKeyConfigured(false);
      showNotice(message, "error");
    }
  }

  function submitPromptFromMessage({
      prompt,
      attachments,
      aspectRatio,
      resolution,
      imageCount,
      clearComposer,
    }: RunPromptOptions) {
    return ensureXaiKeyReady().then((ready) => {
      if (!ready) return;
      return runPrompt({
        prompt,
        attachments,
        aspectRatio,
        resolution,
        imageCount,
        clearComposer,
      });
    });
  }

  async function retryMessage(messageId: string | undefined) {
    const message = messagesStateRef.current.find((item) => item.id === messageId);
    if (!message || message.role !== "user" || !message.content || !message.attachments.length) return;
    const nextAspect = message.aspectRatio || aspectRatio;
    const nextResolution = message.resolution || resolution;
    const nextCount = normalizeImageCount(message.imageCount);
    setAspectRatio(nextAspect);
    setResolution(nextResolution);
    setImageCount(nextCount);
    await submitPromptFromMessage({
      prompt: message.content,
      attachments: message.attachments,
      aspectRatio: nextAspect,
      resolution: nextResolution,
      imageCount: nextCount,
      clearComposer: false,
    });
  }

  async function runPrompt({ prompt, attachments, aspectRatio, resolution, imageCount, clearComposer }: RunPromptOptions) {
    const userAttachments = attachments.map((item) => ({ ...item }));
    const normalizedImageCount = normalizeImageCount(imageCount);
    const userMessage = createMessage("user", prompt, userAttachments, {
      aspectRatio,
      resolution,
      imageCount: normalizedImageCount,
    });
    const pendingMessage = createMessage("assistant", "", [], { pending: true });

    clearNotice();
    setSelectedAttachments(userAttachments.map((item) => ({ ...item })));
    setMessages((current) => [
      ...current,
      userMessage,
      pendingMessage,
    ]);
    setPending(true);
    if (clearComposer) setComposerText("");

    try {
      const sourceDataUris = await Promise.all(userAttachments.map(imageInputForRequest));

      const result = await editImage({
        sourcePaths: userAttachments.map((item) => item.path),
        sourceDataUris,
        prompt,
        aspectRatio: aspectRatio === "auto" ? null : aspectRatio,
        resolution,
        imageCount: normalizedImageCount,
      });

      const resultPaths = Array.isArray(result.paths) && result.paths.length ? result.paths : [result.path];
      const outputAttachments = resultPaths
        .filter(Boolean)
        .map((path) => createAttachment({ path, dataUrl: convertFileSrc(path) }))
        .filter(Boolean) as Attachment[];
      setMessages((current) => current.map((item) => (
        item.id === pendingMessage.id ? createMessage("assistant", "", outputAttachments) : item
      )));
      setSelectedAttachments(outputAttachments.slice(0, MAX_REFERENCES));
      setPending(false);
    } catch (error) {
      if (isXaiKeyStatusError(error)) {
        setMessages((current) => current.filter((item) => (
          item.id !== userMessage.id && item.id !== pendingMessage.id
        )));
        setPending(false);
        notifyXaiKeyError(error);
        throw error;
      }
      setMessages((current) => current.map((item) => (
        item.id === pendingMessage.id
          ? createMessage("assistant", formatErrorMessage(error, "生成失败"), [], { tone: "error" })
          : item
      )));
      setPending(false);
      throw error;
    }
  }

  function flashMessageAction(messageId: string | undefined, feedback: string, duration: number) {
    if (!messageId) return;
    setActionFeedback((current) => ({ ...current, [messageId]: feedback }));
    window.setTimeout(() => {
      setActionFeedback((current) => {
        if (current[messageId] !== feedback) return current;
        const next = { ...current };
        delete next[messageId];
        return next;
      });
    }, duration);
  }

  async function copyMessage(messageId: string | undefined) {
    const message = messagesStateRef.current.find((item) => item.id === messageId);
    if (!message?.content) return;
    setComposerText(message.content);
    window.requestAnimationFrame(() => inputRef.current?.focus());
    await navigator.clipboard.writeText(message.content);
  }

  function handleAttachmentContextMenu(event: MouseEvent, attachment: Attachment) {
    event.preventDefault();
    if (pending) return;
    setSelectedAttachments((current) => toggleAttachment(current, attachment));
  }

  const rootClass = classNames("editor-drawer-root", open && "is-open", closing && "is-closing");
  const ariaHidden = open || closing ? "false" : "true";

  return (
    <div id="editor-drawer" className={rootClass} aria-hidden={ariaHidden}>
      <button className="editor-drawer-backdrop" type="button" aria-label="关闭" onClick={closeDrawer} />
      <aside className="editor-drawer-panel" role="dialog" aria-modal="true" aria-label="编辑">
        <div className="editor-drawer-shell">
          <EditorNotice notice={notice} />
          <section className="editor-messages" ref={messagesRef} aria-live="polite" onScroll={handleMessagesScroll}>
            {messages.map((message) => (
              <MessageView
                key={message.id}
                message={message}
                feedback={actionFeedback[message.id] || ""}
                pending={pending}
                onCopy={(id) => {
                  flashMessageAction(id, "copied", COPY_FEEDBACK_MS);
                  copyMessage(id).catch((error) => onError(error, "复制失败"));
                }}
                onRetry={(id) => {
                  flashMessageAction(id, "retrying", RETRY_FEEDBACK_MS);
                  retryMessage(id).catch((error) => onError(error, "重试失败"));
                }}
                onPreview={onPreviewAttachment}
                onAttachmentContextMenu={handleAttachmentContextMenu}
              />
            ))}
          </section>
          <div className="editor-composer">
            <div className="editor-control-row" role="group" aria-label="编辑选项">
              {ASPECT_BUTTONS.map((button) => {
                const selected = button.values.includes(aspectRatio as never);
                const activeValue = selected ? aspectRatio : button.values[0];
                return (
                  <button
                    key={button.key}
                    className={classNames("editor-setting-button", selected && "is-active")}
                    type="button"
                    aria-pressed={selected}
                    disabled={pending}
                    onMouseDown={(event) => event.preventDefault()}
                    onClick={() => applyAspectToggle(button.key)}
                  >
                    {button.kind === "single" ? button.label : activeValue}
                  </button>
                );
              })}
              {RESOLUTION_BUTTONS.map((button) => {
                const selected = resolution === button.value;
                return (
                  <button
                    key={button.value}
                    className={classNames("editor-setting-button editor-setting-button-resolution", selected && "is-active")}
                    type="button"
                    aria-pressed={selected}
                    disabled={pending}
                    onMouseDown={(event) => event.preventDefault()}
                    onClick={() => toggleResolution(button.value)}
                  >
                    {button.label}
                  </button>
                );
              })}
              <button
                className="editor-setting-button editor-setting-button-count is-active"
                type="button"
                aria-label={`图片数量 ${imageCount} 张`}
                disabled={pending}
                onMouseDown={(event) => event.preventDefault()}
                onClick={cycleImageCount}
              >
                {imageCount}张
              </button>
            </div>
            <form className="editor-input-wrap editor-input-form" onSubmit={handleSubmit}>
              <div className={classNames("editor-selected-files", selectedAttachments.length > 0 && "has-files")}>
                {selectedAttachments.map((attachment) => (
                  <button
                    className="editor-selected-card"
                    type="button"
                    key={attachment.path}
                    aria-label="移除图片"
                    disabled={pending}
                    onClick={() => setSelectedAttachments((current) => current.filter((item) => item.path !== attachment.path))}
                  >
                    <img src={sourceForAttachment(attachment)} alt="" loading="lazy" decoding="async" draggable={false} />
                  </button>
                ))}
              </div>
              <div className="editor-input-row">
                <textarea
                  id="editor-message-input"
                  ref={inputRef}
                  rows={1}
                  autoComplete="off"
                  placeholder="输入提示词"
                  disabled={pending}
                  value={composerText}
                  onChange={(event) => setComposerText(event.currentTarget.value)}
                  onKeyDown={handleKeyDown}
                />
                <div className="editor-input-actions">
                  <button className="editor-icon-button" type="button" aria-label="添加图片" disabled={pending} onClick={() => handlePickReferences().catch((error) => onError(error, "添加参考图失败"))}>
                    <Icons.Plus />
                  </button>
                  <button className="editor-send-button" type="submit" aria-label="发送" disabled={pending}>
                    <Icons.ArrowUp />
                  </button>
                </div>
              </div>
            </form>
          </div>
        </div>
      </aside>
    </div>
  );

  function imageInputForRequest(attachment: Attachment) {
    const source = attachment.dataUrl || attachment.dataUri || "";
    if (isXaiImageInput(source)) return Promise.resolve(source);
    return readImageDataUri(attachment.path);
  }
});

async function persistSession(sessionId: string, messages: Message[]) {
  await invoke("save_editor_session", {
    sessionId,
    messages: serializeMessages(messages),
  });
}

function EditorNotice({ notice }: { notice: EditorNoticeState }) {
  if (!notice.message) return null;
  return (
    <div className="editor-notice" data-tone={notice.tone} role="status" aria-live="polite">
      {notice.message}
    </div>
  );
}

function MessageView({
  message,
  feedback,
  pending,
  onCopy,
  onRetry,
  onPreview,
  onAttachmentContextMenu,
}: {
  message: Message;
  feedback: string;
  pending: boolean;
  onCopy: (id: string) => void;
  onRetry: (id: string) => void;
  onPreview: (attachment: Attachment) => void;
  onAttachmentContextMenu: (event: MouseEvent, attachment: Attachment) => void;
}) {
  const bubble = <MessageBubble message={message} />;
  const actions = message.role === "user" && !message.pending && message.content ? (
    <div className="editor-message-actions">
      <button
        className={classNames("editor-message-action-button", feedback === "copied" && "is-copied")}
        type="button"
        aria-label="复制"
        disabled={pending}
        onClick={() => onCopy(message.id)}
      >
        <Icons.ClipboardDocument />
      </button>
      <button
        className={classNames("editor-message-action-button", feedback === "retrying" && "is-retrying")}
        type="button"
        aria-label="重试"
        disabled={pending}
        onClick={() => onRetry(message.id)}
      >
        <Icons.ArrowPath />
      </button>
    </div>
  ) : null;
  const attachments = message.attachments.length ? (
    <div className="editor-message-attachments">
      {message.attachments.map((attachment) => (
        <AttachmentPreview
          key={attachment.path}
          attachment={attachment}
          onPreview={onPreview}
          onContextMenu={onAttachmentContextMenu}
        />
      ))}
    </div>
  ) : null;

  return (
    <article className={`editor-message editor-message-${message.role}`}>
      {message.role === "user" ? (
        <>
          {attachments}
          {bubble}
          {actions}
        </>
      ) : (
        <>
          {bubble}
          {actions}
          {attachments}
        </>
      )}
    </article>
  );
}

function AttachmentPreview({
  attachment,
  onPreview,
  onContextMenu,
}: {
  attachment: Attachment;
  onPreview: (attachment: Attachment) => void;
  onContextMenu: (event: MouseEvent, attachment: Attachment) => void;
}) {
  const [missing, setMissing] = useState(false);
  return (
    <button
      className={classNames("editor-attachment-card", missing && "is-missing")}
      type="button"
      aria-label={missing ? "图片不可用" : "预览图片"}
      disabled={missing}
      onClick={() => onPreview(attachment)}
      onContextMenu={(event) => onContextMenu(event, attachment)}
    >
      {missing ? (
        <span className="editor-attachment-missing">图片已移动或不存在</span>
      ) : (
        <img
          src={sourceForAttachment(attachment)}
          alt=""
          loading="lazy"
          decoding="async"
          draggable={false}
          onError={() => setMissing(true)}
        />
      )}
    </button>
  );
}

function MessageBubble({ message }: { message: Message }) {
  if (message.role === "assistant") {
    if (message.pending) {
      return (
        <div className="editor-response editor-response-pending">
          <span className="editor-loading-label">生成中</span>
          <span className="editor-thinking-dots" aria-hidden="true"><i /><i /><i /></span>
        </div>
      );
    }
    if (!message.content) return null;
    return <div className={classNames("editor-response", message.tone && `editor-response-${message.tone}`)}>{message.content}</div>;
  }

  if (message.pending) {
    return (
      <div className="editor-bubble editor-bubble-pending">
        <span className="editor-loading-label">发送中</span>
        <span className="editor-thinking-dots" aria-hidden="true"><i /><i /><i /></span>
      </div>
    );
  }
  if (!message.content) return null;
  return <div className={classNames("editor-bubble", message.tone && `editor-bubble-${message.tone}`)}>{message.content}</div>;
}

function createAttachment(item: Partial<Attachment> | null | undefined): Attachment | null {
  if (!item?.path) return null;
  return {
    path: item.path,
    label: item.label || mediaName(item.path),
    dataUrl: item.dataUrl || item.dataUri || "",
  };
}

function createMessage(role: Role, content: string, attachments: Attachment[] = [], extra: Partial<Message> = {}): Message {
  return {
    id: `${Date.now()}-${Math.random().toString(16).slice(2)}`,
    role,
    content: String(content || ""),
    attachments: attachments.map((item) => ({ ...item })),
    pending: extra.pending === true,
    tone: extra.tone || "",
    aspectRatio: extra.aspectRatio || null,
    resolution: extra.resolution || null,
    imageCount: normalizeImageCount(extra.imageCount),
  };
}

function sourceForAttachment(attachment: Attachment) {
  return attachment.dataUrl || attachment.dataUri || convertFileSrc(attachment.path || "");
}

function normalizePersistedMessages(messages: Message[]) {
  return messages
    .filter((message) => message.role === "user" || message.role === "assistant")
    .map<Message>((message) => ({
      ...message,
      pending: false,
      tone: message.tone === "error" ? "error" : "",
      attachments: normalizePersistedAttachments(message.attachments),
      imageCount: normalizeImageCount(message.imageCount),
    }));
}

function normalizePersistedAttachments(attachments: Attachment[]) {
  return (Array.isArray(attachments) ? attachments : [])
    .filter((attachment) => attachment?.path)
    .map((attachment) => ({
      path: attachment.path,
      label: attachment.label || mediaName(attachment.path),
    }));
}

function serializeMessages(messages: Message[]) {
  return messages
    .filter((message) => !message.pending)
    .map((message) => ({
      id: message.id,
      role: message.role,
      content: message.content,
      attachments: normalizePersistedAttachments(message.attachments),
      tone: message.tone || null,
      aspectRatio: message.aspectRatio || null,
      resolution: message.resolution || null,
      imageCount: normalizeImageCount(message.imageCount),
    }));
}

function normalizePickedReferences(picked: PickedImage[]) {
  return (Array.isArray(picked) ? picked : picked ? [picked] : [])
    .map((item) => createAttachment({ path: item.path, dataUri: item.dataUri }))
    .filter(Boolean) as Attachment[];
}

function dedupeAttachments(items: Attachment[]) {
  const byPath = new Map<string, Attachment>();
  for (const item of items) {
    if (!item?.path) continue;
    byPath.set(item.path, { ...item, label: item.label || mediaName(item.path) });
  }
  return Array.from(byPath.values()).slice(0, MAX_REFERENCES);
}

function toggleAttachment(current: Attachment[], attachment: Attachment) {
  const exists = current.some((item) => item.path === attachment.path);
  if (exists) return current.filter((item) => item.path !== attachment.path);
  return dedupeAttachments([...current, attachment]);
}

function isXaiImageInput(value: string) {
  return /^data:image\/[a-z0-9.+-]+;base64,/iu.test(value) || /^https:\/\//iu.test(value);
}

function isXaiKeyStatusError(error: unknown) {
  const message = rawErrorMessage(error);
  return /xai\s*key|api\s*key|key\s*(未设置|无效|失效|missing|empty|invalid|expired)/iu.test(message);
}

function resizeTextarea(element: HTMLTextAreaElement | null) {
  if (!element) return;
  element.style.height = "0px";
  element.style.height = `${Math.min(element.scrollHeight, 160)}px`;
  element.style.overflowY = element.scrollHeight > 160 ? "auto" : "hidden";
}

function isNearMessagesBottom(element: HTMLDivElement) {
  return element.scrollHeight - element.scrollTop - element.clientHeight <= BOTTOM_STICK_THRESHOLD;
}

function scrollMessagesToBottom(element: HTMLDivElement) {
  element.scrollTo({ top: element.scrollHeight });
  shouldStickToBottomIfAtBottom(element);
}

function shouldStickToBottomIfAtBottom(element: HTMLDivElement) {
  if (isNearMessagesBottom(element)) {
    element.scrollTop = element.scrollHeight;
  }
}

function nextAspectRatio(key: string | undefined, currentValue: string) {
  const button = ASPECT_BUTTONS.find((item) => item.key === key);
  if (!button) return null;
  if (button.kind === "single") return button.values[0];
  const activeIndex = button.values.indexOf(currentValue as never);
  if (activeIndex >= 0) return button.values[(activeIndex + 1) % button.values.length];
  return button.values[0];
}

function normalizeImageCount(value: unknown) {
  const count = Number(value);
  if (!Number.isFinite(count)) return 1;
  return Math.min(4, Math.max(1, Math.round(count)));
}

function nextImageCount(currentValue: number) {
  const currentIndex = IMAGE_COUNT_VALUES.indexOf(normalizeImageCount(currentValue));
  if (currentIndex < 0) return IMAGE_COUNT_VALUES[0];
  return IMAGE_COUNT_VALUES[(currentIndex + 1) % IMAGE_COUNT_VALUES.length];
}
