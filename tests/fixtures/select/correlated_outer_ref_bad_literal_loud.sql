-- A correlated outer reference compared against an **unparseable** literal. Once the
-- enclosing level types `cb_o.ouid` as `uuid`, the literal gets its coercing cast and
-- `validate_const_casts` rejects it at plan time — `22P02`, the same code and the same
-- plan-time moment Postgres reports it.
--
-- Before the chaining this was `42883` ("operator does not exist: uuid = text"): the
-- reference was never typed, so the *eval-time* gate spoke instead of the cast. The
-- error is the load-bearing half of the fix — widening the scope must produce
-- Postgres's error, not merely stop producing pglet's.
-- @setup
CREATE TABLE cb_o (okey INT, ouid UUID);
CREATE TABLE cb_i (ikey INT);
INSERT INTO cb_o VALUES (1, '11111111-1111-1111-1111-111111111111');
INSERT INTO cb_i VALUES (1);
-- @expect-error: unparseable literal against a correlated uuid column — 22P02
-- @test
SELECT okey FROM cb_o WHERE EXISTS (SELECT 1 FROM cb_i WHERE cb_o.ouid = 'not-a-uuid');
