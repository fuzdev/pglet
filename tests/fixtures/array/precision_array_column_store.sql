-- The same element-wise rounding on the **store** path: the catalog holds the
-- element's precision (`timestamp(3)[]`'s `atttypmod` is `3` on 18.6) and
-- `coerce_column_value` walks the elements.
-- @setup
CREATE TABLE tpa (a timestamp(3)[], b interval(3)[]);
INSERT INTO tpa VALUES ('{2020-01-02 03:04:05.6789}', '{1.23456 secs}');
-- @test
SELECT a, b FROM tpa;
