import { NextRequest, NextResponse } from "next/server";
import { isAuthRequired, verifySessionToken, SESSION_COOKIE_NAME } from "@/lib/auth-session";

export async function GET(req: NextRequest) {
  const authRequired = isAuthRequired();
  if (!authRequired) {
    return NextResponse.json({ authRequired: false, authenticated: true });
  }

  const token = req.cookies.get(SESSION_COOKIE_NAME)?.value;
  const authenticated = verifySessionToken(token);
  return NextResponse.json({ authRequired: true, authenticated });
}
