-- `COPY … FROM STDIN` reaches the same store finalizer as `INSERT`, so a column's
-- declared sub-second precision rounds the ingested value rather than storing it whole.
-- @setup
CREATE TABLE t (
  id int,
  a timestamp(3),
  b time(3),
  c interval(3),
  d timestamptz(3)
);
-- @copy-from
COPY t FROM STDIN;
-- @data
1	2020-01-02 03:04:05.6789	03:04:05.6789	1.23456 secs	2020-01-02 03:04:05.6789+00
2	1999-12-31 23:59:59.9995	23:59:59.9999	-0.0015 secs	1999-12-31 23:59:59.9995+00
\.
-- @verify
SELECT id, a, b, c, d FROM t ORDER BY id;
