-- `COPY … FROM STDIN` applies an `hh:mm:ss` offset to a scalar field and to an array
-- element.
-- @setup
CREATE TABLE t (id int, tz timestamptz, tzs timestamptz[], ts timestamp);
-- @copy-from
COPY t FROM STDIN;
-- @data
1	2000-01-01 00:00:00+05:30:15	{"2000-01-01 00:00:00-15:59:59"}	2000-01-01 00:00:00+05:30:15
-- @verify
SELECT id, tz, tzs, ts FROM t ORDER BY id;
