FROM ghcr.io/astral-sh/uv:python3.12-bookworm-slim
WORKDIR /app
COPY pyproject.toml /app/pyproject.toml
COPY uv.lock /app/uv.lock
COPY src /app/src
RUN uv sync
ENV PORT=3000
EXPOSE 3000
CMD ["uv", "run", "visit-counter-api"]
