-- The shape a real consumer writes: a `text` column compared against the session
-- role. It is also the row-level evidence that the comparison resolves rather than
-- matching nothing — the two-row table means a silently-false predicate answers
-- zero rows instead of the one that is correct.
-- @setup
CREATE TABLE si_owned (id INT, owner TEXT);
INSERT INTO si_owned VALUES (1, 'postgres'), (2, 'somebody_else');
-- @test
SELECT id FROM si_owned WHERE owner = current_user ORDER BY id;
