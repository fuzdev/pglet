-- `nullif` over an aggregate and a `text` column resolves no `=` operator: 18.6's
-- `42883`, not an answer from a cast of the aggregate to `text`.
-- @setup
CREATE TABLE r (i INT, tx TEXT);
INSERT INTO r VALUES (7, '7'), (3, 'a');
-- @expect-error: operator does not exist: bigint = text
-- @test
SELECT nullif(sum(i), tx) FROM r GROUP BY tx;
