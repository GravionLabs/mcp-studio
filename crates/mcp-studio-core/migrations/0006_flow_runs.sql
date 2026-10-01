-- A run of a flow: its inputs, what every step did, and every tool call with its result, so a run
-- can be inspected and replayed later.
CREATE TABLE flow_runs (
  id          TEXT PRIMARY KEY,
  flow_id     TEXT,
  flow_name   TEXT NOT NULL,
  -- The flow as it was when the run started; editing the flow later does not change the run.
  flow        TEXT NOT NULL,
  inputs      TEXT NOT NULL,
  status      TEXT NOT NULL,
  started_at  INTEGER NOT NULL,
  ended_at    INTEGER,
  outputs     TEXT,
  error       TEXT,
  -- Set when the run replays another run's tool results.
  replay_of   TEXT
);
CREATE INDEX flow_runs_started ON flow_runs (started_at);

CREATE TABLE flow_run_steps (
  run_id      TEXT NOT NULL REFERENCES flow_runs (id) ON DELETE CASCADE,
  seq         INTEGER NOT NULL,
  step_id     TEXT NOT NULL,
  kind        TEXT NOT NULL,
  status      TEXT NOT NULL,
  started_at  INTEGER NOT NULL,
  ended_at    INTEGER,
  -- What the step was asked to do after templates were filled in (prompt, arguments, ...).
  resolved    TEXT,
  output      TEXT,
  error       TEXT,
  span_id     TEXT,
  PRIMARY KEY (run_id, seq)
);

CREATE TABLE flow_run_calls (
  run_id      TEXT NOT NULL REFERENCES flow_runs (id) ON DELETE CASCADE,
  seq         INTEGER NOT NULL,
  step_id     TEXT NOT NULL,
  server      TEXT NOT NULL,
  tool        TEXT NOT NULL,
  arguments   TEXT NOT NULL,
  result      TEXT,
  is_error    INTEGER NOT NULL DEFAULT 0,
  -- The user did not allow the call; it never ran.
  denied      INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (run_id, seq)
);
