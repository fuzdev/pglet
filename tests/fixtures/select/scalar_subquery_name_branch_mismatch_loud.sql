-- A branch the coercion pass cannot wrap — `name` is no cast's target — is still a type
-- the **gate** reads, so the branch set is refused rather than answered under whichever
-- branch's type the walkers reached first.
-- @setup
CREATE TABLE ssgl (i INT, d DATE);
INSERT INTO ssgl VALUES (1, '2020-01-02');
-- @test
SELECT COALESCE((SELECT current_user), i, d) FROM ssgl;
-- @expect-error: COALESCE types name and integer cannot be matched
