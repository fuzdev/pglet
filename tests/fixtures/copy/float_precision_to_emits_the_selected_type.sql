-- `COPY … TO STDOUT` emits the stored value, so the bytes carry the width the column's
-- selected type gave it. `b`/`c` are the controls at the other side of the 24/25 split
-- and at no modifier at all.
-- @setup
CREATE TABLE t (id int, a FLOAT(1), b FLOAT(25), c FLOAT);
INSERT INTO t VALUES
  (1, 0.1234567890123456, 0.1234567890123456, 0.1234567890123456),
  (2, -0.1234567890123456, -0.1234567890123456, -0.1234567890123456);
-- @copy-to
COPY t TO STDOUT;
