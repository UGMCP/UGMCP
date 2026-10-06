interface Props {
  onClick: () => void;
}

export function PlusTile({ onClick }: Props) {
  return (
    <div className="add-cell">
      <button className="plus-tile" aria-label="New workspace" onClick={onClick}>
        <span className="plus-glyph" />
      </button>
    </div>
  );
}
