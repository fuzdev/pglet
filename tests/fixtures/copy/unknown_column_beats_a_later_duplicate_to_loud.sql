-- The other position of the interleave: an unknown name ahead of the repeat is reached
-- first, so the answer is `42703`. Both engines agreed here before the duplicate gate
-- existed, and a gate written as a whole-list pass ahead of the resolution reddens this
-- file rather than its sibling.
-- @setup
CREATE TABLE cdtu (a int, b int);
INSERT INTO cdtu VALUES (1, 2);
-- @copy-to
COPY cdtu (zz, a, a) TO STDOUT;
