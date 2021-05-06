# Asynchronous mutable variable

Based on haskell's [Control.Concurrent.MVar](https://hackage.haskell.org/package/base-4.15.0.0/docs/Control-Concurrent-MVar.html). Per its documentation:

> An `MVar t` is mutable location that is either empty or contains a value of type `t`. It has two fundamental operations: `putMVar` which fills an MVar if it is empty and blocks otherwise, and `takeMVar` which empties an MVar if it is full and blocks otherwise.


