-- `COPY … TO STDOUT` writes an infinity as the word, with no `+00` on a `timestamptz`.
-- @setup
CREATE TABLE t (id int, d date, ts timestamp(3), tz timestamptz, ds date[]);
INSERT INTO t VALUES (1, 'infinity', '-infinity', 'infinity', '{-infinity,infinity}'), (2, '-infinity', 'infinity', '-infinity', NULL);
-- @copy-to
COPY t TO STDOUT;
