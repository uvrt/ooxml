#!/bin/bash
# TEMPORARY (lane L): outside the loop's process: a run going for more than $1 seconds (its
# out/current.json not rewritten) is a hang -- two Python dumps 20 s apart (SIGUSR1) with each
# thread's CPU ticks, the native stacks (gdb), then the process killed.
limit=$1
while sleep 15; do
  [ -f out/current.json ] || continue
  pid=$(python3 -c "import json;print(json.load(open('out/current.json'))['pid'])" 2>/dev/null) || continue
  kill -0 "$pid" 2>/dev/null || exit 0
  age=$(( $(date +%s) - $(stat -c %Y out/current.json) ))
  if [ "$age" -gt "$limit" ]; then
    f=out/hang-external-$(date +%s).txt
    cat out/current.json > $f
    for k in 1 2; do
      echo "== cpu ticks (tid comm utime stime)" >> $f
      for t in /proc/$pid/task/*; do echo "$(basename $t) $(cat $t/comm) $(awk '{print $14, $15}' $t/stat)" >> $f; done
      kill -USR1 $pid; sleep 20; echo "== python stacks (SIGUSR1) so far" >> $f; cat out/stacks.txt >> $f
    done
    echo "== native" >> $f
    sudo gdb -iex "set debuginfod enabled off" -p $pid -batch -ex "set pagination off" -ex "info threads" -ex "thread apply all bt 50" >> $f 2>&1
    kill -9 $pid
    exit 0
  fi
done
