-- The refusal half of `check_constraint_member_position`, and the one that catches a fix
-- that made the CHECK accept everything: `happy` is declared after `ok`, so
-- `CHECK (m < 'ok')` refuses it (`23514`) — where a label-byte comparison accepts it,
-- `happy` being lexically below `ok`. The setup cannot fail, so the asserted error is
-- the `@test`'s own.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE c (id INT, m mood, CHECK (m < 'ok'));

-- @test
-- @expect-error: happy is declared after ok, so the CHECK refuses it
INSERT INTO c VALUES (1, 'happy');
