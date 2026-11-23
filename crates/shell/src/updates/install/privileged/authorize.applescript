on run arguments
    set commandText to "/usr/bin/osascript -l JavaScript -e " & quoted form of (item 1 of arguments)
    repeat with argument in items 2 thru -1 of arguments
        set commandText to commandText & " " & quoted form of (contents of argument)
    end repeat
    try
        do shell script commandText with administrator privileges
    on error messageText number errorNumber
        if errorNumber is -128 or errorNumber is -60006 then return "QUICKIE_UPDATE_CANCELLED"
        error messageText number errorNumber
    end try
end run
