interface Props {
  title: string;
  primary: string;
  secondary: string;
  busy?: boolean;
  error?: string | null;
  onPrimary: () => void;
  onSecondary: () => void;
  onCancel?: () => void;
}

export function LoginScreen({ title, primary, secondary, busy, error, onPrimary, onSecondary, onCancel }: Props) {
  return (
    <section className="login">
      <h1 className="wordmark">{title}</h1>
      <button className="primary" onClick={onPrimary} disabled={busy}>
        {busy ? "Waiting for browser…" : primary}
      </button>
      {busy && onCancel ? (
        <button className="linkish" onClick={onCancel}>
          Cancel
        </button>
      ) : (
        <button className="linkish" onClick={onSecondary}>
          {secondary}
        </button>
      )}
      <p className="hint" role="status">
        {error ?? ""}
      </p>
    </section>
  );
}
