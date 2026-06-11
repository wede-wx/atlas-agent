interface EmptyStateProps {
  title: string;
  body: string;
}

export function EmptyState({ title, body }: EmptyStateProps) {
  return (
    <div className="empty-state">
      <div className="empty-orb">◇</div>
      <h2>{title}</h2>
      <p>{body}</p>
    </div>
  );
}
