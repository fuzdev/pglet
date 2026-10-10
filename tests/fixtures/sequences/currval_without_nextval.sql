-- currval before any nextval in this session is undefined: SQLSTATE 55000
-- (object_not_in_prerequisite_state). The sequence exists, it just hasn't been
-- advanced here.
-- @setup
CREATE SEQUENCE seq_currval_undef;
-- @test
SELECT currval('seq_currval_undef');
-- @expect-error: currval not yet defined in this session (55000)
