-- `SKIP LOCKED` decides nothing before the projection is resolved: 18.6 refuses the
-- projection `42703`, and so does pglet.
-- @expect-error: the projection error stands beside a wait policy
-- @setup
CREATE TABLE lskipc (id INT, code TEXT);
INSERT INTO lskipc VALUES (1, 'a');
-- @test
SELECT nosuchcol FROM lskipc FOR UPDATE SKIP LOCKED;
