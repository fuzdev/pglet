-- The column walk is ordered, so which of two bad columns answers is the **first** one:
-- a refused length before an unrecognized type name is the `22023`, and its mirror
-- (`refused_typmod_column_walk_names_the_first_loud`) is the `42704`. A walk that
-- collected verdicts and picked by kind would be green on one of the pair and red on the
-- other.
-- @expect-error: the first column's refused length, not the second column's bad type
-- @expect-message
-- @test
CREATE TABLE rtw1 (a CHAR(0), b nosuchtype);
