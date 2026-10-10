-- A malformed date string compared against a DATE column fails loud (`22007`,
-- invalid_datetime_format), matching an explicit `'notadate'::date`.
-- @setup
CREATE TABLE t (d DATE);
INSERT INTO t VALUES ('2024-06-15');
-- @test
-- @expect-error: malformed date vs date column errors 22007
SELECT * FROM t WHERE d = 'notadate';
