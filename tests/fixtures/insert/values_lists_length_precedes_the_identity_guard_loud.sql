-- 18.6 raises the `GENERATED ALWAYS` guard in the rewriter, after every row's parse
-- analysis, so a later row's length wins over the first row's identity value: `42601`, not
-- `428C9`.
-- @setup
CREATE TABLE vlig (id INT GENERATED ALWAYS AS IDENTITY, b INT);

-- @test
-- @expect-error: VALUES lists must all be the same length (42601), ahead of the identity guard's 428C9
-- @expect-message
INSERT INTO vlig (id, b) VALUES (1, 2), (3);
