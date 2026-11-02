"use client";

import { Bar, BarChart, Tooltip, XAxis, YAxis } from "recharts";
import { CustomTooltip } from "@/components/CustomTooltip";
import type { MonthlyTotal } from "@/lib/analysis";

export function AnalysisChart({ data }: { data: MonthlyTotal[] }) {
  return (
    <BarChart width={600} height={300} data={data}>
      <XAxis dataKey="month" />
      <YAxis />
      <Tooltip content={<CustomTooltip />} />
      <Bar dataKey="total" />
    </BarChart>
  );
}
