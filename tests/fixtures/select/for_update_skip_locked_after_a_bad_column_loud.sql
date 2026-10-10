-- The refusal is **deferred** to the statement's end, so every error 18.6 does raise on
-- such a statement outranks it. 18.6 accepts `SKIP LOCKED` and refuses the projection
-- `42703`; raised on the spot, pglet's `0A000` would win instead and the SQLSTATEs would
-- part.
-- @expect-error: the projection error outranks the wait-policy refusal
-- @setup
CREATE TABLE lskipc (id INT, code TEXT);
INSERT INTO lskipc VALUES (1, 'a');
-- @test
SELECT nosuchcol FROM lskipc FOR UPDATE SKIP LOCKED;
