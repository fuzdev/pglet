-- The all-`unknown` fold at DDL time, over **member** labels: `CHECK (m IN ('ok','sad'))`
-- is accepted on both engines (`check_ddl_in_list_members_accept`) while the `= ANY`
-- spelling of the same labels is `42883` on both, the array constructor having resolved
-- them to `text` before the operator. The two spellings disagreeing is 18.6's, not this
-- gate's.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE w (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok'), (2, 'ok', 'ok'), (3, 'happy', 'zzz');

-- @test
-- @expect-error: no operator for mood = text, an all-unknown array being text[]
-- @expect-message
CREATE TABLE ck (m mood NOT NULL, CHECK (m = ANY(ARRAY['ok', 'sad'])));
