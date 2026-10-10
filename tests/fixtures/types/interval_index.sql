-- A B-tree index over an INTERVAL column orders on the estimate; a range predicate
-- (`>= 10 days AND < 40 days`) uses it and includes `1 mon` (30 days on the estimate).
-- @profile-lock
-- @setup
CREATE TABLE iv_idx (id INT, span INTERVAL);
CREATE INDEX iv_idx_span ON iv_idx (span);
INSERT INTO iv_idx VALUES (1, INTERVAL '1 mon'), (2, INTERVAL '10 days'),
                          (3, INTERVAL '40 days'), (4, INTERVAL '2 hours');
-- @test
SELECT id FROM iv_idx WHERE span >= INTERVAL '10 days' AND span < INTERVAL '40 days' ORDER BY span ASC;
