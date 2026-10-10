-- The `ARRAY[…]` spelling of the same refusal, and the one whose **code** moves: with the
-- branch set left unjudged the element cast ran and failed at eval (`22P02`), a phase
-- later than 18.6's parse-analysis `42804` and under a descriptor already sent.
-- @setup
CREATE TABLE ssar (i INT);
INSERT INTO ssar VALUES (1);
-- @test
SELECT ARRAY[(SELECT current_user), i] FROM ssar;
-- @expect-error: ARRAY types name and integer cannot be matched
