import os, time
print("peekaboo daily", "DATABASE_URL" in os.environ, flush=True)
time.sleep(3600)
