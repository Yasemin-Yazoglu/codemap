import { getReceipts } from "@/lib/db";

export async function GET() {
  return Response.json(await getReceipts());
}

export async function POST(req: Request) {
  return Response.json(await req.json(), { status: 201 });
}
