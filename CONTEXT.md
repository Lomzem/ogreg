# Register access

This glossary describes register access through a device's text command interface.

## Language

**Register read**:
A request to retrieve the value of a register at a specified address.

**Register write**:
A request to set the value of a register at a specified address.

**Reported value**:
A register value printed by the device. It does not establish that the underlying hardware access succeeded.

**Register polling**:
Repeated reads of a register at a user-specified interval.

**Command acknowledgement**:
A device response that acknowledges a text command request. It does not establish that the requested register operation succeeded.
_Avoid_: Operation success

**Printed output**:
Text emitted by the device, which may include command results or unrelated diagnostics. Printed output does not inherently identify the command that caused it.
_Avoid_: Command response
