import type { Receipt } from "@/lib/db";

export type MonthlyTotal = { month: string; total: number };

export function buildMonthlyData(receipts: Receipt[]): MonthlyTotal[] {
  const totals = new Map<string, number>();
  for (const r of receipts) {
    const month = r.date.slice(0, 7);
    totals.set(month, (totals.get(month) ?? 0) + r.amount);
  }
  return [...totals].map(([month, total]) => ({ month, total }));
}
