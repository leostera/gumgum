FROM ghcr.io/astral-sh/uv:python3.12-bookworm-slim
WORKDIR /app
COPY pyproject.toml /app/pyproject.toml
COPY src /app/src
COPY migrations /app/migrations
RUN uv sync --no-dev
CMD ["uv", "run", "visit-counter-worker"]
