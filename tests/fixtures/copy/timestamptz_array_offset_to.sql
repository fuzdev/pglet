-- `COPY … TO STDOUT` writes each finite `timestamptz[]` element with its `+00`, so a
-- restore in a session at another zone reads the instant stored; a `timestamp[]`
-- element carries none.
-- @setup
CREATE TABLE t (id int, tzs timestamptz[], tss timestamp[]);
INSERT INTO t VALUES (1, '{"2000-01-01 00:00:00+05",NULL,infinity}', '{"2000-01-01 00:00:00+05",NULL}'), (2, ARRAY[timestamptz '2001-06-15 12:30:45.5-03:30'], NULL);
-- @copy-to
COPY t TO STDOUT;
