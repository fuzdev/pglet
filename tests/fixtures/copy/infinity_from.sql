-- `COPY … FROM STDIN` reads the `infinity` spellings for each type, into an indexed column
-- and an array, and the rows land as Postgres stores them.
-- @setup
CREATE TABLE t (id int, d date PRIMARY KEY, ts timestamp, tz timestamptz, ds date[]);
CREATE INDEX t_tz ON t (tz);
-- @copy-from
COPY t FROM STDIN;
-- @data
1	infinity	-infinity	INFINITY	{infinity,-infinity}
2	-infinity	infinity	-infinity	{2000-01-01}
3	2000-01-01	\N	\N	\N
\.
-- @verify
SELECT id, d, ts, tz, ds, isfinite(d) FROM t WHERE tz <= timestamptz 'infinity' OR tz IS NULL ORDER BY d;
