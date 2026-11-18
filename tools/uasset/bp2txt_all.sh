#!/bin/bash
# Regenerate extracted/bp/*.txt for the player blueprints. usage: bp2txt_all.sh [path/to/bp2txt.exe]
X=${1:-target/release/bp2txt.exe}; E=${BODYCAM_EXTRACTED:-$(dirname "$0")/../../extracted}; O=$E/bp; M=$E/Bodycam.usmap; C=$E/Bodycam/Content
P=$E/Bodycam/Plugins/BodycamAnimationFramework/Content/Blueprints
mkdir -p $O
{
  echo $P/Bodycam_Player.uasset; echo $P/BP_BodycamPlayerCameraManager.uasset
  find $P/Animation/Components -name "AC_Bodycam*.uasset"
  find $P/Notifies -name "*.uasset"
  echo $C/AdvancedLocomotionV4/Blueprints/CharacterLogic/ALS_Base_CharacterBP.uasset
  find $C/BodycamCore/AbilitySystem/Abilities -name "GA_*.uasset"
  echo $C/BodycamWeapons/Core/Blueprint/WEP.uasset; echo $C/BodycamWeapons/Core/Blueprint/BP_EquippableItem.uasset
} > $O/_list.txt
: > $O/_summary.log
while read f; do
  b=$(basename "$f" .uasset)
  $X $M "$f" > "$O/$b.txt" 2>>$O/_summary.log || echo "FAIL $f" >> $O/_summary.log
done < $O/_list.txt
sed 's|.*/||' $O/_summary.log | sort
