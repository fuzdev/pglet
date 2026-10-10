-- A comparison of strings is no certain decider, and a function's value is not trusted
-- to be 18.6's: `upper('ß')` is `SS` by Unicode's mapping and `ß` under the oracle's
-- locale, where the comparison is false and the `sum` runs. Deciding by the value
-- computed here would answer `t`.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT upper('ß') = 'SS' OR sum(1/b) > 0 FROM u;
