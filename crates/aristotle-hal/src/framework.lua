-- SPDX-License-Identifier: AGPL-3.0-or-later
local lipc = require('liblipclua')
local handle = assert(lipc.init('com.lab126.kaf'), 'KAF name unavailable')
local started = assert(handle:register_int_property('frameworkStarted', 'r'))
started.value = 1
io.write('aristotle-framework-ready\n')
io.flush()
while true do
    lipc.run_event_loop(1000)
end
