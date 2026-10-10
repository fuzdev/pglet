-- The enum spelling of the earlier-row literal: `'zz'` is not a label of `vle_mood`, and
-- 18.6 refuses it in the first row's assignment, through the enum's input function — `22P02`,
-- ahead of the second row's length.
-- @setup
CREATE TYPE vle_mood AS ENUM ('ok', 'sad');
CREATE TABLE vlee (m vle_mood, b INT);

-- @test
-- @expect-error: invalid input value for enum vle_mood: "zz" (22P02), ahead of the second row's 42601
-- @expect-message
INSERT INTO vlee VALUES ('zz', 1), ('ok');
