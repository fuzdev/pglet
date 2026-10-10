-- The parenthesized `now()` spelling reduces onto the same closed `Now` default the
-- bare form takes, so the column carries a real timestamp rather than being refused.
-- Asserted as a comparison rather than a value, since the instant is not stable.
-- @setup
CREATE TABLE dpn (k INT, t TIMESTAMPTZ DEFAULT (now()), d DATE DEFAULT (CURRENT_DATE));
INSERT INTO dpn (k) VALUES (1);
-- @test
SELECT k, t IS NOT NULL AS t_set, d IS NOT NULL AS d_set FROM dpn;
