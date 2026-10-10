-- An `ON CONFLICT DO UPDATE` queues an inserted row's key check beside the rows' actions.
-- The inserted `(4, 1)` is checked in its place, ahead of the action the next row's move of
-- key `1` queues, and finds `1` gone.
-- @setup
CREATE TABLE p (id int PRIMARY KEY, par int REFERENCES p ON UPDATE CASCADE);
INSERT INTO p VALUES (1, NULL);
-- @test
-- @expect-error: the inserted row's check runs before the later row's action
-- @expect-message
-- @profile-lock
INSERT INTO p VALUES (4, 1), (1, NULL) ON CONFLICT (id) DO UPDATE SET id = 10;
