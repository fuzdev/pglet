-- `COPY … FROM STDIN` reads a `timestamptz` field through the type's reader, so an
-- offset past 15:59:59 is refused rather than stored at a shifted instant.
-- @setup
CREATE TABLE t (id int, tz timestamptz);
-- @copy-from
COPY t FROM STDIN;
-- @data
1	2000-01-01 00:00:00+16
-- @verify
SELECT id, tz FROM t ORDER BY id;
