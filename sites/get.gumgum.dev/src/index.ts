export interface Env {
  GUMGUM_RELEASES: R2Bucket;
}

const contentTypes: Record<string, string> = {
  ".sh": "text/x-shellscript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".gz": "application/gzip",
};

function objectKey(url: URL): string {
  let path = url.pathname;
  if (path === "/" || path === "") return "install.sh";
  path = path.replace(/^\/+/, "");
  if (path === "install" || path === "install.sh") return "install.sh";
  return path;
}

function contentTypeFor(key: string): string {
  for (const [suffix, contentType] of Object.entries(contentTypes)) {
    if (key.endsWith(suffix)) return contentType;
  }
  return "application/octet-stream";
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    if (request.method !== "GET" && request.method !== "HEAD") {
      return new Response("method not allowed\n", { status: 405 });
    }

    const url = new URL(request.url);
    const key = objectKey(url);
    const object = await env.GUMGUM_RELEASES.get(key);

    if (!object) {
      return new Response(`not found: ${key}\n`, { status: 404 });
    }

    const headers = new Headers();
    object.writeHttpMetadata(headers);
    headers.set("etag", object.httpEtag);
    headers.set("cache-control", key === "install.sh" ? "no-cache" : "public, max-age=31536000, immutable");
    if (!headers.has("content-type")) headers.set("content-type", contentTypeFor(key));

    return new Response(request.method === "HEAD" ? null : object.body, { headers });
  },
};
