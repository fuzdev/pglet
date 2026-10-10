-- `COPY … TO STDOUT` emits the **stored** value, so the bytes carry the rounding the
-- column's precision applied on the way in.
-- @setup
CREATE TABLE t (
  id int,
  a timestamp(3),
  b time(3),
  c interval(3),
  d timestamptz(3)
);
INSERT INTO t VALUES
  (1, '2020-01-02 03:04:05.6789', '03:04:05.6789', '1.23456 secs', '2020-01-02 03:04:05.6789+00'),
  (2, '1999-12-31 23:59:59.9995', '23:59:59.9999', '-0.0015 secs', '1999-12-31 23:59:59.9995+00');
-- @copy-to
COPY t TO STDOUT;
