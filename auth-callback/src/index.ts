import express, { type Request, type Response } from "express";

const app = express();

const GITHUB_CLIENT_ID = process.env.GITHUB_CLIENT_ID!;
const GITHUB_CLIENT_SECRET = process.env.GITHUB_CLIENT_SECRET!;
const PORT = process.env.PORT || 8080;

const ALLOWED_ORIGINS = [
  "https://rerun-io.github.io/kitdiff",
  "https://rerun-io.github.io/kitdiff/",
];

function isAllowedRedirect(url: string): boolean {
  if (ALLOWED_ORIGINS.includes(url)) {
    return true;
  }
  try {
    const parsed = new URL(url);
    return (
      (parsed.hostname === "127.0.0.1" || parsed.hostname === "localhost") &&
      parsed.protocol === "http:"
    );
  } catch {
    return false;
  }
}

app.get("/callback", async (req: Request, res: Response) => {
  const { code, state } = req.query;

  if (typeof code !== "string" || typeof state !== "string") {
    res.status(400).send("Missing code or state parameter");
    return;
  }

  if (!isAllowedRedirect(state)) {
    res.status(400).send("Invalid redirect URL");
    return;
  }

  try {
    const response = await fetch(
      "https://github.com/login/oauth/access_token",
      {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Accept: "application/json",
        },
        body: JSON.stringify({
          client_id: GITHUB_CLIENT_ID,
          client_secret: GITHUB_CLIENT_SECRET,
          code,
        }),
      },
    );

    const data = (await response.json()) as {
      access_token?: string;
      error?: string;
      error_description?: string;
    };

    if (data.error) {
      res.status(400).send(`GitHub error: ${data.error_description}`);
      return;
    }

    const redirectUrl = `${state}#token=${data.access_token}`;
    res.redirect(302, redirectUrl);
  } catch (err) {
    console.error("Token exchange failed:", err);
    res.status(500).send("Token exchange failed");
  }
});

app.get("/health", (_req: Request, res: Response) => {
  res.send("ok");
});

app.listen(PORT, () => {
  console.log(`Auth callback listening on port ${PORT}`);
});
