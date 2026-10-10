-- With the enum on the **array** side the left-hand operand is the `unknown`, and the
-- element's enum is what 18.6 coerces it to. Its accept-side twin is
-- `any_all_array_side_member_position`, where the same spelling with a member label
-- answers on both engines.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: zzz is not a member of mood
-- @expect-message
SELECT id FROM w WHERE 'zzz' = ANY(ARRAY[m]);
