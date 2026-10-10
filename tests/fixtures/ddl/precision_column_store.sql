-- A column's declared precision is enforced on **store**, at every write path — the
-- catalog holds it and `coerce_column_value` is the one finalizer every one of them
-- funnels through. `INSERT` and `UPDATE` here; the upsert, backfill, `DEFAULT VALUES`,
-- `RETURNING` and `COPY` spellings have their own fixtures.
-- @setup
CREATE TABLE tps (id int, a timestamp(3), b time(3), c interval(3), d timestamptz(3));
INSERT INTO tps VALUES (1, '2020-01-02 03:04:05.6789', '03:04:05.6789', '1.23456 secs', '2020-01-02 03:04:05.6789');
INSERT INTO tps VALUES (2, '1999-12-31 23:59:59.9995', '23:59:59.9999', '-0.0015 secs', '1999-12-31 23:59:59.9995');
UPDATE tps SET a = '2000-01-01 00:00:00.0005' WHERE id = 2;
-- @test
SELECT id, a, b, c, d FROM tps ORDER BY id;
