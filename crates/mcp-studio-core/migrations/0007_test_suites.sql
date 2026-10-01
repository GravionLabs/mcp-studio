-- Test suites: a prompt (optional) and cases that say which tool a model should choose for an
-- input, or what its answer should contain. Suites belong to a server.
CREATE TABLE test_suites (
  id             TEXT PRIMARY KEY,
  server_id      TEXT NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
  name           TEXT NOT NULL,
  system_prompt  TEXT,
  updated_at     INTEGER NOT NULL
);
CREATE INDEX test_suites_server ON test_suites (server_id);

CREATE TABLE test_cases (
  id           TEXT PRIMARY KEY,
  suite_id     TEXT NOT NULL REFERENCES test_suites (id) ON DELETE CASCADE,
  position     INTEGER NOT NULL,
  input        TEXT NOT NULL,
  -- JSON: {"kind":"tool","name":...}, {"kind":"noTool"} or {"kind":"answer","contains":...}
  expectation  TEXT NOT NULL,
  notes        TEXT
);
CREATE INDEX test_cases_suite ON test_cases (suite_id, position);
