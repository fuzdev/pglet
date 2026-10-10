-- `INSERT … DEFAULT VALUES` resolves the stored default and stores it through the same
-- finalizer, so the row carries the rounded value even though the recorded default
-- literal is the one that was written (see
-- `information_schema/precision_column_default_literal.sql`).
-- @setup
CREATE TABLE tpd (a timestamp(3) DEFAULT '2020-01-02 03:04:05.6789',
                  b time(3) DEFAULT '03:04:05.6789');
INSERT INTO tpd DEFAULT VALUES;
-- @test
SELECT a, b FROM tpd;
