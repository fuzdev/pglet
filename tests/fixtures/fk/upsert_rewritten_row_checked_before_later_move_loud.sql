-- An `ON CONFLICT DO UPDATE` queues a rewritten row's key check beside the rows' actions.
-- The first row newly references key `1`, which exists when the row is written; its queued
-- check runs ahead of the action the second row's move of key `1` queues, and finds `1`
-- gone.
-- @setup
CREATE TABLE p (id int PRIMARY KEY, par int REFERENCES p ON UPDATE CASCADE);
INSERT INTO p VALUES (1, NULL), (2, NULL);
-- @test
-- @expect-error: the rewritten row's check runs before the later row's action
-- @expect-message
-- @profile-lock
INSERT INTO p VALUES (2, 0), (1, 0) ON CONFLICT (id) DO UPDATE SET par = CASE WHEN p.id = 2 THEN 1 ELSE p.par END, id = CASE WHEN p.id = 1 THEN 10 ELSE p.id END;
