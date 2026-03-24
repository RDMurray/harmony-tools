// functions/_middleware.ts
// Password-only gate for an entire Cloudflare Pages site.
// - GET /__login shows a password form
// - POST /__login verifies password, sets a signed cookie, redirects
// - GET /__logout clears cookie
// Everything else requires the cookie.

interface Env {
  SITE_PASSWORD: string;
}

const LOGIN_PATH = "/__login";
const LOGOUT_PATH = "/__logout";
const COOKIE_NAME = "__Host-pages_pw"; // __Host- requires Secure + Path=/ and no Domain
const COOKIE_TTL_SECONDS = 60 * 60 * 24 * 7; // 7 days

export async function onRequest(context: any) {
  const { request, env } = context as { request: Request; env: Env };
  const url = new URL(request.url);

  // Basic safety: ensure password secret is present
  if (!env?.SITE_PASSWORD) {
    return new Response("Server not configured: missing SITE_PASSWORD", {
      status: 500,
      headers: { "Cache-Control": "no-store" },
    });
  }

  // Logout
  if (url.pathname === LOGOUT_PATH) {
    return new Response(null, {
      status: 302,
      headers: {
        Location: LOGIN_PATH,
        "Set-Cookie": `${COOKIE_NAME}=; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=0`,
        "Cache-Control": "no-store",
      },
    });
  }

  // Login routes
  if (url.pathname === LOGIN_PATH) {
    if (request.method === "GET") {
      return loginPage(url.searchParams.get("r") ?? "/", false);
    }

    if (request.method === "POST") {
      const form = await request.formData();
      const password = String(form.get("password") ?? "");
      const redirectTo = String(form.get("r") ?? "/") || "/";

      if (password !== env.SITE_PASSWORD) {
        return loginPage(redirectTo, true);
      }

      const token = await mintToken(env.SITE_PASSWORD, COOKIE_TTL_SECONDS);
      return new Response(null, {
        status: 302,
        headers: {
          Location: safeRedirect(redirectTo),
          "Set-Cookie": `${COOKIE_NAME}=${token}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=${COOKIE_TTL_SECONDS}`,
          "Cache-Control": "no-store",
        },
      });
    }

    return new Response("Method Not Allowed", { status: 405 });
  }

  // For everything else: check auth cookie
  const token = getCookie(request, COOKIE_NAME);
  const ok = token ? await verifyToken(env.SITE_PASSWORD, token) : false;

  if (ok) {
    return context.next();
  }

  // If it's a browser navigation, redirect to login; otherwise return 401.
  const accept = request.headers.get("Accept") ?? "";
  const fetchMode = request.headers.get("Sec-Fetch-Mode") ?? "";
  const isNavigate = fetchMode === "navigate" || accept.includes("text/html");

  if (isNavigate) {
    const r = url.pathname + url.search;
    return new Response(null, {
      status: 302,
      headers: {
        Location: `${LOGIN_PATH}?r=${encodeURIComponent(r)}`,
        "Cache-Control": "no-store",
      },
    });
  }

  return new Response("Unauthorized", {
    status: 401,
    headers: { "Cache-Control": "no-store" },
  });
}

function loginPage(redirectTo: string, wrong: boolean) {
  const body = `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width,initial-scale=1" />
  <title>Protected</title>
  <meta name="robots" content="noindex,nofollow" />
  <style>
    body{font-family:system-ui,-apple-system,Segoe UI,Roboto,Arial,sans-serif;margin:0;display:grid;place-items:center;min-height:100vh;padding:24px}
    .card{max-width:360px;width:100%;border:1px solid #ddd;border-radius:12px;padding:18px}
    label{display:block;margin:10px 0 6px}
    input{width:100%;font-size:16px;padding:10px;border:1px solid #ccc;border-radius:10px}
    button{margin-top:14px;width:100%;font-size:16px;padding:10px;border-radius:10px;border:1px solid #111;background:#111;color:#fff}
    .err{color:#b00020;margin:8px 0 0}
  </style>
</head>
<body>
  <form class="card" method="POST" action="${LOGIN_PATH}">
    <h1 style="margin:0 0 6px;font-size:20px">Enter password</h1>
    <p style="margin:0 0 10px;color:#555">This site is private.</p>
    <input type="hidden" name="r" value="${escapeHtml(redirectTo)}" />
    <label for="password">Password</label>
    <input id="password" name="password" type="password" autocomplete="current-password" autofocus />
    ${wrong ? `<div class="err">Incorrect password</div>` : ``}
    <button type="submit">Continue</button>
  </form>
</body>
</html>`;

  return new Response(body, {
    status: wrong ? 401 : 200,
    headers: {
      "Content-Type": "text/html; charset=utf-8",
      "Cache-Control": "no-store",
    },
  });
}

function safeRedirect(r: string) {
  // only allow same-site relative redirects
  if (!r.startsWith("/")) return "/";
  if (r.startsWith("//")) return "/";
  return r;
}

function getCookie(request: Request, name: string) {
  const cookie = request.headers.get("Cookie") ?? "";
  const parts = cookie.split(/;\s*/);
  for (const part of parts) {
    const idx = part.indexOf("=");
    if (idx < 0) continue;
    const k = part.slice(0, idx);
    const v = part.slice(idx + 1);
    if (k === name) return v;
  }
  return null;
}

async function mintToken(secret: string, ttlSeconds: number) {
  const exp = Math.floor(Date.now() / 1000) + ttlSeconds;
  const msg = String(exp);
  const sig = await hmacHex(secret, msg);
  return `${exp}.${sig}`;
}

async function verifyToken(secret: string, token: string) {
  const [expStr, sig] = token.split(".");
  const exp = Number(expStr);
  if (!Number.isFinite(exp)) return false;
  if (exp < Math.floor(Date.now() / 1000)) return false;

  const expected = await hmacHex(secret, expStr);
  return timingSafeEqual(sig, expected);
}

async function hmacHex(secret: string, message: string) {
  const enc = new TextEncoder();
  const key = await crypto.subtle.importKey(
    "raw",
    enc.encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"]
  );
  const sig = await crypto.subtle.sign("HMAC", key, enc.encode(message));
  return toHex(new Uint8Array(sig));
}

function toHex(bytes: Uint8Array) {
  let out = "";
  for (const b of bytes) out += b.toString(16).padStart(2, "0");
  return out;
}

function timingSafeEqual(a: string, b: string) {
  if (a.length !== b.length) return false;
  let r = 0;
  for (let i = 0; i < a.length; i++) r |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return r === 0;
}

function escapeHtml(s: string) {
  return s
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}