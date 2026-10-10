-- The second cross-route control: `ALTER COLUMN … SET DEFAULT` also only *stores*, so
-- the same `1/0` is accepted there and raises at the first omitting write. The two
-- controls together are what make the `ADD COLUMN` refusals a statement about the
-- route rather than about the expression.
-- @setup
CREATE TABLE sd_deferred (a INT, b INT);
ALTER TABLE sd_deferred ALTER COLUMN b SET DEFAULT 1/0;
INSERT INTO sd_deferred (a, b) VALUES (1, 7);
-- @test
SELECT a, b FROM sd_deferred;
