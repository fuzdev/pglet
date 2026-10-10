-- COPY ... TO STDOUT must escape the control bytes Postgres does: a tab, newline,
-- and backslash inside a text value render as \t, \n, \\ (so the row framing
-- survives). Byte-for-byte against Postgres.
-- @setup
CREATE TABLE w (id int, body text);
INSERT INTO w VALUES
  (1, E'has\ttab'),
  (2, E'has\nnewline'),
  (3, E'has\\backslash'),
  (4, E'all\t\n\\three');
-- @copy-to
COPY w TO STDOUT;
