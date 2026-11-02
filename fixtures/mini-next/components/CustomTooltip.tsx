export function CustomTooltip({ active, payload }: { active?: boolean; payload?: { value: number }[] }) {
  if (!active || !payload?.length) return null;
  return <div>{payload[0].value.toFixed(2)} €</div>;
}
