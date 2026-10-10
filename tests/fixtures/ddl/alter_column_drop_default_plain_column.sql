-- The other polarity of the identity gate on `ALTER COLUMN … SET/DROP DEFAULT`: a
-- plain column's default is still the user's to set and to clear. A gate written
-- against the *statement* rather than the target column refuses this too, and every
-- `@expect-error` fixture beside it stays green while it does.
-- @setup
CREATE TABLE acddp_t (a INT DEFAULT 7, b INT);
ALTER TABLE acddp_t ALTER COLUMN b SET DEFAULT 9;
INSERT INTO acddp_t (a, b) VALUES (DEFAULT, DEFAULT);
ALTER TABLE acddp_t ALTER COLUMN a DROP DEFAULT;
INSERT INTO acddp_t (a, b) VALUES (DEFAULT, DEFAULT);
-- @test
SELECT a, b FROM acddp_t ORDER BY b, a;
